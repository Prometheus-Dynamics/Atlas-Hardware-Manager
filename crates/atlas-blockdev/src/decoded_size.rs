//! How large a compressed image is once decoded, read from the format's own
//! headers and trailers without decoding it, so a flash that can't fit is
//! refused before the first write. Each answer is a lower bound: the last
//! xz stream's index, the first zstd frame's content size, the last gzip
//! member's ISIZE (the size modulo 4 GiB). A file of several streams, frames
//! or members can be larger, never smaller, so a refusal is always right;
//! an image the headers don't size is still stopped by the writer when it
//! runs out of disk.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::ImageFormat;

/// At least how many bytes `path` decodes to, when its format says.
pub(crate) fn decoded_at_least(path: &Path, format: ImageFormat) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    match format {
        ImageFormat::Raw => file.metadata().ok().map(|meta| meta.len()),
        ImageFormat::Gzip => gzip_isize(&mut file),
        ImageFormat::Zstd => zstd_content_size(&mut file),
        ImageFormat::Xz => xz_uncompressed(&mut file),
    }
}

fn gzip_isize(file: &mut File) -> Option<u64> {
    let mut tail = [0u8; 4];
    file.seek(SeekFrom::End(-4)).ok()?;
    file.read_exact(&mut tail).ok()?;
    Some(u64::from(u32::from_le_bytes(tail)))
}

/// The first frame's Frame_Content_Size (RFC 8878 3.1.1.1), when written.
fn zstd_content_size(file: &mut File) -> Option<u64> {
    let mut head = [0u8; 18];
    let read = file.read(&mut head).ok()?;
    let head = &head[..read];
    let descriptor = *head.get(4)?;
    let fcs_flag = descriptor >> 6;
    let single_segment = descriptor & 0x20 != 0;
    let dict_bytes = [0usize, 1, 2, 4][usize::from(descriptor & 0x03)];
    let field = match (fcs_flag, single_segment) {
        (0, false) => return None,
        (0, true) => 1,
        (1, _) => 2,
        (2, _) => 4,
        _ => 8,
    };
    let start = 5 + usize::from(!single_segment) + dict_bytes;
    let bytes = head.get(start..start + field)?;
    let mut value = [0u8; 8];
    value[..field].copy_from_slice(bytes);
    let size = u64::from_le_bytes(value);
    Some(if field == 2 { size + 256 } else { size })
}

/// The sum of the last stream's records in its index (xz file format 4, 6).
fn xz_uncompressed(file: &mut File) -> Option<u64> {
    let len = file.metadata().ok()?.len();
    // Stream padding: zero bytes in fours after the footer.
    let mut end = len;
    let mut word = [0u8; 4];
    while end >= 12 + 4 {
        file.seek(SeekFrom::Start(end - 4)).ok()?;
        file.read_exact(&mut word).ok()?;
        if word != [0; 4] {
            break;
        }
        end -= 4;
    }
    let mut footer = [0u8; 12];
    file.seek(SeekFrom::Start(end.checked_sub(12)?)).ok()?;
    file.read_exact(&mut footer).ok()?;
    if &footer[10..12] != b"YZ" {
        return None;
    }
    let backward = u64::from(u32::from_le_bytes(footer[4..8].try_into().ok()?));
    let index_size = (backward + 1) * 4;
    let index_start = end.checked_sub(12 + index_size)?;
    let mut index = vec![0u8; usize::try_from(index_size).ok()?];
    file.seek(SeekFrom::Start(index_start)).ok()?;
    file.read_exact(&mut index).ok()?;
    if index.first() != Some(&0) {
        return None;
    }
    let mut at = 1;
    let records = vli(&index, &mut at)?;
    let mut total = 0u64;
    for _ in 0..records {
        let _unpadded = vli(&index, &mut at)?;
        total = total.checked_add(vli(&index, &mut at)?)?;
    }
    Some(total)
}

/// An xz variable-length integer: seven bits a byte, low first, at most nine.
fn vli(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in 0..9 {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7F) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("atlas-decoded-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn xz_and_gzip_say_their_decoded_size() {
        let original = vec![7u8; 300_000];
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut &original[..], &mut xz).unwrap();
        let path = temp("xz", &xz);
        assert_eq!(decoded_at_least(&path, ImageFormat::Xz), Some(300_000));
        let _ = std::fs::remove_file(path);

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&original).unwrap();
        let path = temp("gz", &gz.finish().unwrap());
        assert_eq!(decoded_at_least(&path, ImageFormat::Gzip), Some(300_000));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_zstd_frame_header_gives_its_content_size() {
        // Magic, descriptor (8-byte FCS, single segment), the size.
        let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0b1110_0000];
        frame.extend_from_slice(&(16u64 << 30).to_le_bytes());
        let path = temp("zst", &frame);
        assert_eq!(decoded_at_least(&path, ImageFormat::Zstd), Some(16 << 30));
        let _ = std::fs::remove_file(path);
        // No size written: unknown, not zero.
        let path = temp("zst0", &[0x28, 0xB5, 0x2F, 0xFD, 0b0000_0000, 0x58]);
        assert_eq!(decoded_at_least(&path, ImageFormat::Zstd), None);
        let _ = std::fs::remove_file(path);
    }
}
