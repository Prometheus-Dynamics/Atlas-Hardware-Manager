//! Streaming xz for first-flash images.
//!
//! A first-flash `.img.xz` is one image stream, optionally followed by one more
//! stream that decodes to a 108-byte signature record. Nothing else is accepted:
//! [`scan`] reads the two streams' footers and indexes (by seeking, never decoding
//! the image) and refuses anything else by name. Every reader of an image decodes
//! exactly the image stream ([`decode_image`]): liblzma runs without its
//! concatenation flag, and the input is limited to the image stream's bytes, so a
//! record is never decoded into the image and never written to storage.
//!
//! # Contract points
//!
//! - Dictionary: at most [`DICT_CEILING`] (8 MiB) per block. A board may lower its
//!   own limit; [`dictionary_within`] compares the value, the comparison is the
//!   caller's.
//! - Decoder memory: the dictionary plus [`DECODER_OVERHEAD`] (256 KiB). The
//!   decoder is liblzma, with that bound as its memory limit.
//! - Filters: LZMA2 only. Checks: none, CRC-32, CRC-64, SHA-256 (all four).
//! - Padding: between the two streams, zero bytes in a multiple of four (the xz
//!   stream padding), at most [`MAX_PADDING`]. Padding after the last stream is
//!   refused: trailing bytes are not a stream.
//! - The record stream must decode to at most [`RECORD_MAX`] bytes; whether they
//!   are exactly one record is the caller's, which parses them.
//!
//! liblzma is bundled and linked statically (`xz2`'s `static` feature): the bundled
//! source is liblzma 5.2.5, so the binary needs no liblzma at run time.

use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};

use thiserror::Error;
use xz2::stream::Stream;
use xz2::write::XzDecoder;

/// The dictionary ceiling of the first-flash contract: 8 MiB per block.
pub const DICT_CEILING: u64 = 8 << 20;
/// The decoder's bound over the dictionary: 256 KiB.
pub const DECODER_OVERHEAD: u64 = 256 << 10;
/// The most zero bytes accepted between two streams (a multiple of four).
pub const MAX_PADDING: u64 = 4096;
/// The largest decoded size a second stream may have. A record is 108 bytes.
pub const RECORD_MAX: u64 = 1024;

const XZ_MAGIC: &[u8] = &[0xFD, b'7', b'z', b'X', b'Z', 0x00];
const HEADER: u64 = 12;
const FOOTER: u64 = 12;
/// The largest index read: 1 MiB, about 100k blocks.
const MAX_INDEX: u64 = 1 << 20;
const MAX_BLOCKS: u64 = 8192;
/// The filter id of LZMA2, the only filter read.
const LZMA2: u64 = 0x21;
/// The read buffer for the decode.
const READ_BUFFER: usize = 256 << 10;

/// Why a file is not read as a first-flash image.
#[derive(Debug, Error)]
pub enum XzError {
    /// The file could not be read or seeked.
    #[error("{0}")]
    Io(String),
    /// The file ends before its streams do.
    #[error("the image is truncated: {0}")]
    Truncated(String),
    /// The streams are not what a first-flash image is.
    #[error("not a first-flash image: {0}")]
    Malformed(String),
    /// A block declares more than the contract allows.
    #[error("{0}")]
    Limit(String),
}

/// A byte range of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    pub len: u64,
}

/// The layout of a first-flash file, from its stream footers and indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The image: stream 1, from byte 0 (the whole file for a plain image).
    pub image: Span,
    /// The signature record stream after the image, when there is one.
    pub record: Option<Span>,
    /// The largest dictionary a block of the image declares (0 for a plain image).
    pub dictionary: u64,
    /// The image's decoded size per its index (the file's length for a plain image).
    pub uncompressed: u64,
    /// True for a file that is not xz: the image is the file, as it is.
    pub plain: bool,
}

impl Layout {
    /// The layout of a file that is not xz: the whole file is the image.
    pub fn plain(len: u64) -> Layout {
        Layout {
            image: Span { start: 0, len },
            record: None,
            dictionary: 0,
            uncompressed: len,
            plain: true,
        }
    }
}

/// Whether the file starts with the xz magic. Reads from byte 0 whatever the
/// position, and leaves the reader at byte 0.
pub fn is_xz<R: Read + Seek>(input: &mut R) -> io::Result<bool> {
    input.seek(SeekFrom::Start(0))?;
    let mut magic = [0u8; 6];
    let mut got = 0;
    while got < magic.len() {
        match input.read(&mut magic[got..])? {
            0 => break,
            n => got += n,
        }
    }
    input.seek(SeekFrom::Start(0))?;
    Ok(got == magic.len() && magic == *XZ_MAGIC)
}

/// Whether the image's dictionary fits a limit (a board's, at most the ceiling).
pub fn dictionary_within(layout: &Layout, limit: u64) -> bool {
    layout.dictionary <= limit
}

fn io_error(error: &io::Error) -> XzError {
    XzError::Io(error.to_string())
}

fn le32(bytes: &[u8]) -> Result<u32, XzError> {
    let raw: [u8; 4] = bytes
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| XzError::Malformed("a field is truncated".into()))?;
    Ok(u32::from_le_bytes(raw))
}

/// `count` bytes at `pos`. Callers cap `count` before the call.
fn read_at<R: Read + Seek>(input: &mut R, pos: u64, count: u64) -> Result<Vec<u8>, XzError> {
    input.seek(SeekFrom::Start(pos)).map_err(|e| io_error(&e))?;
    let mut bytes = vec![0u8; count as usize];
    input.read_exact(&mut bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            XzError::Truncated("the file ends inside a stream".into())
        } else {
            io_error(&error)
        }
    })?;
    Ok(bytes)
}

/// An xz variable-length integer at `*at`, advancing it. At most nine bytes.
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, XzError> {
    let mut value = 0u64;
    for shift in (0..63).step_by(7) {
        let byte = *bytes
            .get(*at)
            .ok_or_else(|| XzError::Malformed("a number runs past its index".into()))?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(XzError::Malformed(
        "a number is longer than nine bytes".into(),
    ))
}

/// The CRC-32 the xz headers and footers use (IEEE, reflected).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// One stream as its footer and index give it.
struct StreamInfo {
    start: u64,
    /// The stream flags (footer and header agree on them).
    flags: [u8; 2],
    /// Each block's unpadded size, in order.
    blocks: Vec<u64>,
    /// The decoded size of all blocks, per the index.
    uncompressed: u64,
}

/// Reads the stream that ends at `end`, from its footer and index. Nothing is
/// decoded.
fn stream_ending_at<R: Read + Seek>(input: &mut R, end: u64) -> Result<StreamInfo, XzError> {
    if end < HEADER + FOOTER {
        return Err(XzError::Malformed(
            "the file is shorter than an xz stream".into(),
        ));
    }
    let foot = read_at(input, end - FOOTER, FOOTER)?;
    if foot[10..12] != *b"YZ" {
        return Err(XzError::Malformed(format!(
            "no xz stream ends at byte {end}"
        )));
    }
    if le32(&foot[0..4])? != crc32(&foot[4..10]) {
        return Err(XzError::Malformed(
            "a stream footer's CRC does not match".into(),
        ));
    }
    let flags = [foot[8], foot[9]];
    let index_size = (u64::from(le32(&foot[4..8])?) + 1) * 4;
    if index_size > MAX_INDEX || index_size > end - HEADER - FOOTER {
        return Err(XzError::Malformed(
            "a stream's index is larger than the stream".into(),
        ));
    }
    let index_start = end - FOOTER - index_size;
    let index = read_at(input, index_start, index_size)?;
    let (body, crc) = index.split_at(index.len() - 4);
    if body.first() != Some(&0) {
        return Err(XzError::Malformed(
            "the index does not start where the footer says".into(),
        ));
    }
    if le32(crc)? != crc32(body) {
        return Err(XzError::Malformed("an index's CRC does not match".into()));
    }
    let mut at = 1usize;
    let count = varint(body, &mut at)?;
    if count == 0 || count > MAX_BLOCKS {
        return Err(XzError::Malformed(format!(
            "a stream declares {count} blocks; 1 to {MAX_BLOCKS} are read"
        )));
    }
    let mut blocks = Vec::with_capacity(count as usize);
    let mut uncompressed = 0u64;
    for _ in 0..count {
        let unpadded = varint(body, &mut at)?;
        if unpadded == 0 {
            return Err(XzError::Malformed("a block in an index is empty".into()));
        }
        blocks.push(unpadded);
        let size = varint(body, &mut at)?;
        uncompressed = uncompressed
            .checked_add(size)
            .ok_or_else(|| XzError::Malformed("a stream's decoded size overflows".into()))?;
    }
    if body[at..].iter().any(|byte| *byte != 0) {
        return Err(XzError::Malformed("an index's padding is not zero".into()));
    }
    let mut blocks_total = 0u64;
    for unpadded in &blocks {
        blocks_total = blocks_total
            .checked_add((unpadded + 3) & !3)
            .ok_or_else(|| XzError::Malformed("a stream's blocks overflow".into()))?;
    }
    let start = index_start
        .checked_sub(blocks_total + HEADER)
        .ok_or_else(|| XzError::Malformed("a stream's blocks run past its start".into()))?;
    Ok(StreamInfo {
        start,
        flags,
        blocks,
        uncompressed,
    })
}

/// Checks a stream's header, and each block's header against the index: one LZMA2
/// filter, a dictionary within the ceiling. Returns the largest dictionary.
fn check_stream<R: Read + Seek>(input: &mut R, s: &StreamInfo) -> Result<u64, XzError> {
    let head = read_at(input, s.start, HEADER)?;
    if head[..6] != *XZ_MAGIC {
        return Err(XzError::Malformed(format!(
            "no xz stream header at byte {}",
            s.start
        )));
    }
    if head[6] != 0 {
        return Err(XzError::Malformed(
            "a stream header's reserved flag is not zero".into(),
        ));
    }
    if head[6..8] != s.flags {
        return Err(XzError::Malformed(
            "a stream's footer and header flags differ".into(),
        ));
    }
    // none (0), CRC-32 (1), CRC-64 (4) and SHA-256 (10): all four the contract allows.
    if !matches!(head[7], 0 | 1 | 4 | 10) {
        return Err(XzError::Malformed(format!(
            "a stream's check type {} is not one read",
            head[7]
        )));
    }
    if le32(&head[8..12])? != crc32(&head[6..8]) {
        return Err(XzError::Malformed(
            "a stream header's CRC does not match".into(),
        ));
    }
    let mut dictionary = 0u64;
    let mut at = s.start + HEADER;
    for unpadded in &s.blocks {
        dictionary = dictionary.max(check_block(input, at)?);
        at += (unpadded + 3) & !3;
    }
    Ok(dictionary)
}

/// Checks one block header at `offset` and returns its dictionary.
fn check_block<R: Read + Seek>(input: &mut R, offset: u64) -> Result<u64, XzError> {
    let first = read_at(input, offset, 1)?[0];
    if first == 0 {
        return Err(XzError::Malformed(
            "the index names a block where an index starts".into(),
        ));
    }
    let size = (u64::from(first) + 1) * 4;
    let head = read_at(input, offset, size)?;
    let (body, crc) = head.split_at(head.len() - 4);
    if le32(crc)? != crc32(body) {
        return Err(XzError::Malformed(
            "a block header's CRC does not match".into(),
        ));
    }
    let flags = body[1];
    if flags & 0x03 != 0 {
        return Err(XzError::Malformed(
            "a block has more than one filter; only LZMA2 is read".into(),
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
        return Err(XzError::Malformed(format!(
            "a block's filter is {filter:#x}; only LZMA2 (0x21) is read"
        )));
    }
    let props = varint(body, &mut at)?;
    if props != 1 {
        return Err(XzError::Malformed(
            "an LZMA2 filter's properties are not one byte".into(),
        ));
    }
    let prop = *body
        .get(at)
        .ok_or_else(|| XzError::Malformed("a block header is truncated".into()))?;
    if prop > 40 {
        return Err(XzError::Malformed(format!(
            "the LZMA2 dictionary property {prop} is out of range"
        )));
    }
    let dictionary = (2 | u64::from(prop & 1)) << (u64::from(prop) / 2 + 11);
    if dictionary > DICT_CEILING {
        return Err(XzError::Limit(format!(
            "the xz dictionary is {dictionary} bytes ({} MiB), over the limit of {} MiB ({DICT_CEILING} bytes)",
            dictionary.div_ceil(1 << 20),
            DICT_CEILING >> 20
        )));
    }
    Ok(dictionary)
}

/// The end of the image stream when a record stream starts at `before`: the zero
/// padding before it is skipped, in steps of four, at most [`MAX_PADDING`].
fn image_end_before<R: Read + Seek>(input: &mut R, before: u64) -> Result<u64, XzError> {
    let mut pos = before;
    let mut zeros = 0;
    while pos >= 4 && zeros < MAX_PADDING {
        if read_at(input, pos - 4, 4)? != [0u8; 4] {
            break;
        }
        pos -= 4;
        zeros += 4;
    }
    Ok(pos)
}

/// Reads the layout of the xz file `input`: one image stream from byte 0, and
/// optionally one record stream after it. Each stream's footer and index are
/// checked, and every block header within the contract. Nothing is decoded.
pub fn scan<R: Read + Seek>(input: &mut R) -> Result<Layout, XzError> {
    let len = input.seek(SeekFrom::End(0)).map_err(|e| io_error(&e))?;
    if !is_xz(input).map_err(|e| io_error(&e))? {
        return Err(XzError::Malformed("the file is not an xz stream".into()));
    }
    if len < HEADER + FOOTER || read_at(input, len - FOOTER, FOOTER)?[10..12] != *b"YZ" {
        return Err(XzError::Truncated(
            "the file does not end with an xz stream footer: it is cut, or has bytes after its stream".into(),
        ));
    }
    let last = stream_ending_at(input, len)?;
    let record_candidate =
        last.blocks.len() == 1 && last.uncompressed <= RECORD_MAX && last.start > 0;
    if record_candidate {
        check_stream(input, &last)?;
        let image_end = image_end_before(input, last.start)?;
        let image = stream_ending_at(input, image_end).map_err(|_| {
            XzError::Malformed(
                "the second stream must follow the image's stream directly, after zero padding"
                    .into(),
            )
        })?;
        if image.start != 0 {
            return Err(XzError::Malformed(
                "bytes or another stream come before the image stream".into(),
            ));
        }
        let dictionary = check_stream(input, &image)?;
        return Ok(Layout {
            image: Span {
                start: 0,
                len: image_end,
            },
            record: Some(Span {
                start: last.start,
                len: len - last.start,
            }),
            dictionary,
            uncompressed: image.uncompressed,
            plain: false,
        });
    }
    if last.start != 0 {
        return Err(XzError::Malformed(
            "a second stream is not a signature record, or there is a third stream".into(),
        ));
    }
    let dictionary = check_stream(input, &last)?;
    Ok(Layout {
        image: Span { start: 0, len },
        record: None,
        dictionary,
        uncompressed: last.uncompressed,
        plain: false,
    })
}

/// Decodes the image stream of `layout` into `sink`. For a plain file the bytes are
/// copied. The input is limited to the image's span, so no byte past it is read.
pub fn decode_image<R: Read + Seek, W: Write>(
    input: &mut R,
    layout: &Layout,
    sink: &mut W,
) -> io::Result<()> {
    input.seek(SeekFrom::Start(layout.image.start))?;
    let limited = input.by_ref().take(layout.image.len);
    let mut reader = BufReader::with_capacity(READ_BUFFER, limited);
    if layout.plain {
        io::copy(&mut reader, sink)?;
        return Ok(());
    }
    decode_stream(&mut reader, sink, layout.dictionary)
}

/// The liblzma memory limit for a stream whose dictionary is `dictionary`.
fn memlimit(dictionary: u64) -> u64 {
    dictionary.max(DICT_CEILING) + DECODER_OVERHEAD
}

/// Decodes one xz stream. Concatenated streams are not decoded: liblzma is not
/// asked for them, so the stream's end is the end of its decoding.
fn decode_stream<R: BufRead, W: Write>(
    input: &mut R,
    sink: &mut W,
    dictionary: u64,
) -> io::Result<()> {
    let stream = Stream::new_stream_decoder(memlimit(dictionary), 0).map_err(io::Error::other)?;
    let mut decoder = XzDecoder::new_stream(sink, stream);
    io::copy(input, &mut decoder)?;
    decoder.finish()?;
    Ok(())
}

/// The record stream's decoded bytes (at most [`RECORD_MAX`]), read on their own.
/// Whether they are one record is the caller's to say.
pub fn read_record<R: Read + Seek>(input: &mut R, layout: &Layout) -> Result<Vec<u8>, XzError> {
    let span = layout
        .record
        .ok_or_else(|| XzError::Malformed("the file has no second stream".into()))?;
    input
        .seek(SeekFrom::Start(span.start))
        .map_err(|e| io_error(&e))?;
    let mut bytes = Capped {
        bytes: Vec::new(),
        cap: RECORD_MAX as usize,
    };
    let mut reader = BufReader::new(input.by_ref().take(span.len));
    decode_stream(&mut reader, &mut bytes, DICT_CEILING).map_err(|error| {
        XzError::Malformed(format!("the second stream does not decode: {error}"))
    })?;
    Ok(bytes.bytes)
}

/// A sink that refuses more than `cap` bytes.
struct Capped {
    bytes: Vec<u8>,
    cap: usize,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes.len() + buf.len() > self.cap {
            return Err(io::Error::other(
                "the second stream decodes to more than a record",
            ));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
