//! Image formats, found by their first bytes rather than by file name, and
//! how large an image decodes to, read from its headers.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::DiskError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFormat {
    /// The image as it is: a `.img`.
    Raw,
    /// An `.img.xz`, the form a build produces.
    Xz,
}

const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const ZIP_MAGIC: [u8; 4] = [b'P', b'K', 0x03, 0x04];

/// Looks at the first bytes of `path`. A zip is refused: writing one raw
/// would put the archive on the disk, not the image.
pub fn detect_format(path: &Path) -> Result<ImageFormat, DiskError> {
    let mut head = [0u8; 6];
    let mut file = File::open(path).map_err(|error| DiskError::io(path, error))?;
    let read = file
        .read(&mut head)
        .map_err(|error| DiskError::io(path, error))?;
    let head = &head[..read];
    if head.starts_with(&XZ_MAGIC) {
        Ok(ImageFormat::Xz)
    } else if head.starts_with(&ZIP_MAGIC) {
        Err(DiskError::Image(
            "zip archives are not written directly; extract the .img inside first".into(),
        ))
    } else {
        Ok(ImageFormat::Raw)
    }
}

/// Counts the bytes read from the file, for progress.
struct Counting<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.count.fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

/// Decodes `path` into `sink`, adding to `input_read` as the file is read.
pub(crate) fn decode_into(
    path: &Path,
    format: ImageFormat,
    sink: &mut dyn Write,
    input_read: Arc<AtomicU64>,
) -> io::Result<()> {
    let file = File::open(path)?;
    let mut input = BufReader::with_capacity(
        1 << 20,
        Counting {
            inner: file,
            count: input_read,
        },
    );
    match format {
        ImageFormat::Raw => io::copy(&mut input, sink).map(|_| ()),
        ImageFormat::Xz => {
            let mut sink = sink;
            lzma_rs::xz_decompress(&mut input, &mut sink).map_err(|error| match error {
                lzma_rs::error::Error::IoError(inner) => inner,
                other => io::Error::new(io::ErrorKind::InvalidData, format!("{other:?}")),
            })
        }
    }
}

/// At least how many bytes `path` decodes to, when its format says. Each
/// answer is a lower bound: an xz file of several streams can be larger,
/// never smaller, so refusing on it is always right.
pub(crate) fn decoded_at_least(path: &Path, format: ImageFormat) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    match format {
        ImageFormat::Raw => file.metadata().ok().map(|meta| meta.len()),
        ImageFormat::Xz => xz_uncompressed(&mut file),
    }
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
