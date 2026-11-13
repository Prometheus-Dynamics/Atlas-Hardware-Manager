//! Image formats, found by their first bytes rather than by file name, and
//! how large an image decodes to, read from its headers.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
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

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Decodes `path` into `sink`, adding to `input_read` as the file is read. An
/// xz file is decoded as its image stream only (see `atlas_xzdec`): a signature
/// record after the image is never read, so it is never written to the target.
pub(crate) fn decode_into(
    path: &Path,
    format: ImageFormat,
    sink: &mut dyn Write,
    input_read: Arc<AtomicU64>,
) -> io::Result<()> {
    let file = File::open(path)?;
    let mut input = Counting {
        inner: file,
        count: input_read,
    };
    let layout = match format {
        ImageFormat::Raw => {
            let len = input.seek(SeekFrom::End(0))?;
            input.seek(SeekFrom::Start(0))?;
            atlas_xzdec::Layout::plain(len)
        }
        ImageFormat::Xz => atlas_xzdec::scan(&mut input).map_err(xz_error)?,
    };
    let mut sink = sink;
    atlas_xzdec::decode_image(&mut input, &layout, &mut sink)
}

fn xz_error(error: atlas_xzdec::XzError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// At least how many bytes `path` decodes to, when its format says: the image
/// stream's decoded size from its index. `None` when the layout cannot be read,
/// which the decode then reports in full.
pub(crate) fn decoded_at_least(path: &Path, format: ImageFormat) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut input = Counting {
        inner: file,
        count: Arc::new(AtomicU64::new(0)),
    };
    match format {
        ImageFormat::Raw => input.inner.metadata().ok().map(|meta| meta.len()),
        ImageFormat::Xz => atlas_xzdec::scan(&mut input)
            .ok()
            .map(|layout| layout.uncompressed),
    }
}
