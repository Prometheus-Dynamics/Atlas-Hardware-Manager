//! The xz stream's index, read before anything is decoded.
//!
//! A decoder's memory follows the dictionary each block declares. So before the
//! decoder runs, this module reads
//! the stream's footer and index (near the end of the file, by seeking), checks
//! the stream's flags and the index's CRC, and reads every block header at the
//! offset the index gives. Each block must have exactly one filter, LZMA2, and a
//! dictionary of at most [`MAX_DICT_BYTES`]: the same limit a flasher refuses
//! above (`max_dict_mb`, default 8), and the builder's `xz -6` writes exactly
//! that. The index must also account for every byte between the stream header
//! and the index, so a block the index does not name cannot be decoded unseen.
//!
//! The cost is the header, the footer, the index and one header per block: a few
//! kilobytes for a typical image, read by seeking, never decoded.

use std::io::{Read, Seek, SeekFrom};

use crate::error::ImageError;
use crate::gpt::crc32;

/// The xz stream magic, `FD 37 7A 58 5A 00`.
pub(crate) const XZ_MAGIC: &[u8] = &[0xFD, b'7', b'z', b'X', b'Z', 0x00];
/// The largest dictionary a block may declare: 8 MiB.
pub(crate) const MAX_DICT_BYTES: u64 = 8 << 20;

const HEADER: u64 = 12;
const FOOTER: u64 = 12;
/// The largest index read: 1 MiB, about 100k blocks.
const MAX_INDEX: u64 = 1 << 20;
const MAX_BLOCKS: u64 = 8192;
/// The filter id of LZMA2, the only filter this reader takes.
const LZMA2: u64 = 0x21;

fn io(error: &std::io::Error) -> ImageError {
    ImageError::io("the image", error)
}

fn decode(why: impl Into<String>) -> ImageError {
    ImageError::Decode(why.into())
}

/// Tells whether the image is xz. If it is, its index and block headers are
/// checked (see the module docs). The reader is put back at byte 0 either way.
/// Takes the unbuffered file: the scan seeks and reads small pieces, and a
/// buffered reader would read a whole buffer for each of them.
/// Returns `true` for xz, `false` for a plain image.
pub(crate) fn prepare<R: Read + Seek>(input: &mut R, max_dict: u64) -> Result<bool, ImageError> {
    let mut magic = [0u8; 6];
    let mut got = 0;
    while got < magic.len() {
        match input.read(&mut magic[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(error) => return Err(io(&error)),
        }
    }
    input.seek(SeekFrom::Start(0)).map_err(|error| io(&error))?;
    let xz = got == magic.len() && magic == XZ_MAGIC;
    if xz {
        check(input, max_dict)?;
        input.seek(SeekFrom::Start(0)).map_err(|error| io(&error))?;
    }
    Ok(xz)
}

fn le32(bytes: &[u8]) -> Result<u32, ImageError> {
    let raw: [u8; 4] = bytes
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| decode("a field is truncated"))?;
    Ok(u32::from_le_bytes(raw))
}

/// A `count` of bytes at `pos`. The callers cap `count` first.
fn read_at<R: Read + Seek>(input: &mut R, pos: u64, count: u64) -> Result<Vec<u8>, ImageError> {
    input
        .seek(SeekFrom::Start(pos))
        .map_err(|error| io(&error))?;
    let mut bytes = vec![0u8; count as usize];
    input.read_exact(&mut bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            ImageError::Truncated("the image ends inside its xz stream".into())
        } else {
            io(&error)
        }
    })?;
    Ok(bytes)
}

/// An xz variable-length integer at `*at`, advancing it. At most nine bytes.
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, ImageError> {
    let mut value = 0u64;
    for shift in (0..63).step_by(7) {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| decode("a number runs past the end of its index or block header"))?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(decode("a number is longer than nine bytes"))
}

/// Checks the whole stream's index and every block header. See the module docs.
fn check<R: Read + Seek>(input: &mut R, max_dict: u64) -> Result<(), ImageError> {
    let len = input.seek(SeekFrom::End(0)).map_err(|error| io(&error))?;
    if len < HEADER + FOOTER {
        return Err(ImageError::Truncated(
            "the image is shorter than an xz stream".into(),
        ));
    }

    let head = read_at(input, 0, HEADER)?;
    if head.get(6) != Some(&0) {
        return Err(decode("the stream header's reserved flag is not zero"));
    }
    let check_type = head[7];
    match check_type {
        0 | 1 | 4 => {}
        10 => {
            return Err(decode(
                "the stream uses a SHA-256 check, which this reader does not decode (the builder writes CRC-64)",
            ));
        }
        other => {
            return Err(decode(format!(
                "the stream's check type {other} is not one this reader decodes"
            )));
        }
    }
    if le32(&head[8..12])? != crc32(&head[6..8]) {
        return Err(decode("the stream header's CRC does not match"));
    }

    let foot = read_at(input, len - FOOTER, FOOTER)?;
    if &foot[10..12] != b"YZ" {
        return Err(ImageError::Truncated(
            "the xz footer is missing: the image is cut, or has bytes after its stream".into(),
        ));
    }
    if le32(&foot[0..4])? != crc32(&foot[4..10]) {
        return Err(decode("the stream footer's CRC does not match"));
    }
    if foot[8..10] != head[6..8] {
        return Err(decode("the stream footer's flags differ from its header's"));
    }
    let index_size = (u64::from(le32(&foot[4..8])?) + 1) * 4;
    if index_size > MAX_INDEX || index_size > len - HEADER - FOOTER {
        return Err(decode("the stream's index is larger than the stream"));
    }
    let index_start = len - FOOTER - index_size;

    let index = read_at(input, index_start, index_size)?;
    let (body, crc) = index.split_at(index.len() - 4);
    if body.first() != Some(&0) {
        return Err(decode("the index does not start where the footer says"));
    }
    if le32(crc)? != crc32(body) {
        return Err(decode("the index's CRC does not match"));
    }
    let mut at = 1usize;
    let count = varint(body, &mut at)?;
    if count == 0 || count > MAX_BLOCKS {
        return Err(decode(format!(
            "the stream declares {count} blocks; 1 to {MAX_BLOCKS} are read"
        )));
    }
    let mut unpadded_sizes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        unpadded_sizes.push(varint(body, &mut at)?);
        // The uncompressed size is the decoder's to count; it is read to keep
        // the record layout.
        varint(body, &mut at)?;
    }
    if body[at..].iter().any(|byte| *byte != 0) {
        return Err(decode("the index's padding is not zero"));
    }

    let mut offset = HEADER;
    for unpadded in unpadded_sizes {
        if unpadded == 0 {
            return Err(decode("a block in the index is empty"));
        }
        let total = (unpadded + 3) & !3;
        let end = offset.checked_add(total);
        if end.is_none_or(|end| end > index_start) {
            return Err(decode("a block runs into the index"));
        }
        check_block(input, offset, max_dict)?;
        offset += total;
    }
    if offset != index_start {
        return Err(decode(format!(
            "the blocks end at byte {offset}, but the index starts at {index_start}"
        )));
    }
    Ok(())
}

/// Checks one block header at `offset`: its CRC, one LZMA2 filter, and a
/// dictionary within `max_dict`.
fn check_block<R: Read + Seek>(
    input: &mut R,
    offset: u64,
    max_dict: u64,
) -> Result<(), ImageError> {
    let first = read_at(input, offset, 1)?[0];
    if first == 0 {
        return Err(decode(
            "the index names a block, but an index starts where its header should be",
        ));
    }
    let size = (u64::from(first) + 1) * 4;
    let head = read_at(input, offset, size)?;
    let (body, crc) = head.split_at(head.len() - 4);
    if le32(crc)? != crc32(body) {
        return Err(decode("a block header's CRC does not match"));
    }
    let flags = body[1];
    if flags & 0x03 != 0 {
        return Err(decode(
            "a block has more than one filter; only one LZMA2 filter is read",
        ));
    }
    let mut at = 2usize;
    if flags & 0x40 != 0 {
        varint(body, &mut at)?;
    }
    if flags & 0x80 != 0 {
        varint(body, &mut at)?;
    }
    let filter = varint(body, &mut at)?;
    if filter != LZMA2 {
        return Err(decode(format!(
            "a block's filter is {filter:#x}; only LZMA2 (0x21) is read"
        )));
    }
    let props = varint(body, &mut at)?;
    if props != 1 {
        return Err(decode("an LZMA2 filter's properties are not one byte"));
    }
    let prop = *body
        .get(at)
        .ok_or_else(|| decode("a block header is truncated"))?;
    if prop > 40 {
        return Err(decode(format!(
            "the LZMA2 dictionary property {prop} is out of range"
        )));
    }
    let dict = (2 | u64::from(prop & 1)) << (u64::from(prop) / 2 + 11);
    if dict > max_dict {
        return Err(ImageError::Limit(format!(
            "the xz dictionary is {dict} bytes, more than the {max_dict} bytes this reader takes (8 MiB, the limit a board refuses above)"
        )));
    }
    Ok(())
}
