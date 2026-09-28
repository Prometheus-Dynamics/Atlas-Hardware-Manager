//! Image formats, detected by content rather than file name.

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::BlockError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFormat {
    Raw,
    Xz,
    Zstd,
    Gzip,
}

const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const GZIP_MAGIC: [u8; 2] = [0x1F, 0x8B];
const ZIP_MAGIC: [u8; 4] = [b'P', b'K', 0x03, 0x04];

pub fn detect_format(path: &Path) -> Result<ImageFormat, BlockError> {
    let mut head = [0u8; 6];
    let mut file = File::open(path).map_err(|error| BlockError::io(path, error))?;
    let read = file
        .read(&mut head)
        .map_err(|error| BlockError::io(path, error))?;
    let head = &head[..read];
    Ok(if head.starts_with(&XZ_MAGIC) {
        ImageFormat::Xz
    } else if head.starts_with(&ZSTD_MAGIC) {
        ImageFormat::Zstd
    } else if head.starts_with(&GZIP_MAGIC) {
        ImageFormat::Gzip
    } else if head.starts_with(&ZIP_MAGIC) {
        return Err(BlockError::Image(
            "zip archives are not written directly; extract the .img inside first".into(),
        ));
    } else {
        ImageFormat::Raw
    })
}

/// Counts bytes read from the compressed file, for progress.
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

/// Decodes `path` into `sink`, updating `input_read` as the file is read.
pub(crate) fn decode_into(
    path: &Path,
    format: ImageFormat,
    sink: &mut dyn Write,
    input_read: Arc<AtomicU64>,
) -> Result<(), io::Error> {
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
        ImageFormat::Gzip => {
            io::copy(&mut flate2::read::MultiGzDecoder::new(input), sink).map(|_| ())
        }
        ImageFormat::Zstd => {
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(input)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            io::copy(&mut decoder, sink).map(|_| ())
        }
        ImageFormat::Xz => {
            let mut sink = sink;
            lzma_rs::xz_decompress(&mut input, &mut sink).map_err(|error| match error {
                lzma_rs::error::Error::IoError(inner) => inner,
                other => io::Error::new(io::ErrorKind::InvalidData, format!("{other:?}")),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "atlas-blockdev-image-{label}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn decode(path: &Path) -> Vec<u8> {
        let mut out = Vec::new();
        let format = detect_format(path).unwrap();
        decode_into(path, format, &mut out, Arc::new(AtomicU64::new(0))).unwrap();
        out
    }

    #[test]
    fn raw_and_gzip_decode_to_the_same_bytes() {
        let original: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let raw = temp("raw", &original);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&original).unwrap();
        let gzip = temp("gz", &gz.finish().unwrap());

        assert_eq!(detect_format(&raw).unwrap(), ImageFormat::Raw);
        assert_eq!(detect_format(&gzip).unwrap(), ImageFormat::Gzip);
        assert_eq!(decode(&raw), original);
        assert_eq!(decode(&gzip), original);
        let _ = std::fs::remove_file(raw);
        let _ = std::fs::remove_file(gzip);
    }

    #[test]
    fn xz_decodes() {
        let original = b"atlas image contents ".repeat(500);
        let mut compressed = Vec::new();
        lzma_rs::xz_compress(&mut &original[..], &mut compressed).unwrap();
        let xz = temp("xz", &compressed);

        assert_eq!(detect_format(&xz).unwrap(), ImageFormat::Xz);
        assert_eq!(decode(&xz), original);
        let _ = std::fs::remove_file(xz);
    }

    #[test]
    fn zip_is_refused_with_advice() {
        let zip = temp("zip", b"PK\x03\x04rest");
        assert!(matches!(detect_format(&zip), Err(BlockError::Image(_))));
        let _ = std::fs::remove_file(zip);
    }
}
