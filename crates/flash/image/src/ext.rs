//! The ext2/3/4 side: the data partition's spool file, read with `ext4-view`,
//! which wants random access and is handed the file itself.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use ext4_view::{Ext4, PathBuf as Ext4Path};

use crate::error::ImageError;

fn fs_error(error: impl std::fmt::Display) -> ImageError {
    ImageError::Filesystem(format!("ext4: {error}"))
}

fn to_path(path: &str) -> Result<Ext4Path, ImageError> {
    Ext4Path::try_from(path).map_err(|error| ImageError::InvalidName(format!("{path:?}: {error}")))
}

/// Opens a partition's spool file as an ext4 filesystem.
pub(crate) fn load(spool: &Path) -> Result<Ext4, ImageError> {
    let file = File::open(spool).map_err(|error| ImageError::io("the partition", &error))?;
    Ext4::load(Box::new(file)).map_err(fs_error)
}

/// Whether `path` (absolute, inside the image) is a regular file.
pub(crate) fn is_file(fs: &Ext4, path: &str) -> Result<bool, ImageError> {
    let p = to_path(path)?;
    if !fs.exists(&p).map_err(fs_error)? {
        return Ok(false);
    }
    Ok(fs
        .metadata(&p)
        .map_err(fs_error)?
        .file_type()
        .is_regular_file())
}

/// Opens a regular file of the image. The reader owns what it reads from, so
/// it can outlive the borrow of `fs`.
pub(crate) fn open(fs: &Ext4, path: &str) -> Result<Box<dyn Read>, ImageError> {
    if !is_file(fs, path)? {
        return Err(ImageError::Missing(format!("{path} is not in the image")));
    }
    let file = fs.open(&to_path(path)?).map_err(fs_error)?;
    Ok(Box::new(file))
}

/// The size of a regular file of the image.
pub(crate) fn len(fs: &Ext4, path: &str) -> Result<u64, ImageError> {
    if !is_file(fs, path)? {
        return Err(ImageError::Missing(format!("{path} is not in the image")));
    }
    Ok(fs.metadata(&to_path(path)?).map_err(fs_error)?.len())
}

/// The names of the regular files directly in `dir` (absolute, inside the image),
/// sorted. `None` when the directory is not there. A name that is not UTF-8, or
/// more than `max` files, is refused rather than skipped.
pub(crate) fn list_files(
    fs: &Ext4,
    dir: &str,
    max: usize,
) -> Result<Option<Vec<String>>, ImageError> {
    let path = to_path(dir)?;
    if !fs.exists(&path).map_err(fs_error)? {
        return Ok(None);
    }
    let mut names = Vec::new();
    for entry in fs.read_dir(&path).map_err(fs_error)? {
        let entry = entry.map_err(fs_error)?;
        let file_type = entry.file_type().map_err(fs_error)?;
        if !file_type.is_regular_file() {
            continue;
        }
        let name = entry
            .file_name()
            .as_str()
            .map_err(|_| ImageError::Malformed(format!("a name in {dir} is not UTF-8")))?
            .to_string();
        names.push(name);
        if names.len() > max {
            return Err(ImageError::Limit(format!(
                "{dir} holds more than the {max} files read"
            )));
        }
    }
    names.sort();
    Ok(Some(names))
}

/// A small text file of the image (a verity table), read whole. A file larger
/// than `max` is refused, not read.
pub(crate) fn read_text(fs: &Ext4, path: &str, max: u64) -> Result<String, ImageError> {
    let size = len(fs, path)?;
    if size > max {
        return Err(ImageError::Limit(format!(
            "{path} is {size} bytes, more than the {max} allowed"
        )));
    }
    let bytes = fs.read(&to_path(path)?).map_err(fs_error)?;
    String::from_utf8(bytes).map_err(|_| ImageError::Malformed(format!("{path} is not UTF-8")))
}
