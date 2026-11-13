//! The ext2/3/4 side: a partition's spool file read with `ext4-view`, which
//! wants random access and is handed the file itself. Only regular files and
//! directories are read; links and devices in a slot or boot tree are refused.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use ext4_view::{Ext4, PathBuf as Ext4Path};

use crate::budget::Budget;
use crate::error::ImageError;
use crate::names;
use crate::route::{BootSet, Extracted};

/// Directory entries read from one directory: a cap, so a damaged directory
/// cannot make the walk run on.
const MAX_ENTRIES: usize = 4096;
/// Directory nesting read from a boot tree.
const MAX_DEPTH: u32 = 8;

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

/// Copies the tree under `dir` (absolute, inside the image) into `dest` and
/// returns what it holds, by path under `dir`.
pub(crate) fn extract(
    fs: &Ext4,
    dir: &str,
    dest: &Path,
    budget: &mut Budget,
) -> Result<BootSet, ImageError> {
    if !fs.exists(&to_path(dir)?).map_err(fs_error)? {
        return Err(ImageError::Missing(format!("the rootfs has no {dir}")));
    }
    let mut set = BootSet::new();
    walk(fs, dir, "", 0, dest, budget, &mut set)?;
    Ok(set)
}

fn walk(
    fs: &Ext4,
    dir: &str,
    rel: &str,
    depth: u32,
    dest: &Path,
    budget: &mut Budget,
    set: &mut BootSet,
) -> Result<(), ImageError> {
    if depth > MAX_DEPTH {
        return Err(ImageError::Limit(format!(
            "{dir} nests deeper than {MAX_DEPTH}"
        )));
    }
    let mut entries = Vec::new();
    for entry in fs.read_dir(&to_path(dir)?).map_err(fs_error)? {
        if entries.len() >= MAX_ENTRIES {
            return Err(ImageError::Limit(format!(
                "{dir} holds more than {MAX_ENTRIES} entries"
            )));
        }
        let entry = entry.map_err(fs_error)?;
        let name = entry
            .file_name()
            .as_str()
            .map_err(|_| ImageError::Malformed(format!("a name in {dir} is not UTF-8")))?
            .to_string();
        if name == "." || name == ".." {
            continue;
        }
        names::check_component(&name)?;
        let meta = entry.metadata().map_err(fs_error)?;
        entries.push((name, meta));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, meta) in entries {
        let child = if dir == "/" {
            format!("/{name}")
        } else {
            format!("{dir}/{name}")
        };
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let kind = meta.file_type();
        if kind.is_dir() {
            walk(fs, &child, &child_rel, depth + 1, dest, budget, set)?;
        } else if kind.is_regular_file() {
            let size = meta.len();
            budget.take(size)?;
            let target = dest.join(&child_rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| ImageError::io("the boot files", &error))?;
            }
            let mut source = fs.open(&to_path(&child)?).map_err(fs_error)?;
            let mut out =
                File::create(&target).map_err(|error| ImageError::io("the boot files", &error))?;
            let copied = io::copy(&mut (&mut source).take(size), &mut out)
                .map_err(|error| ImageError::io("the boot files", &error))?;
            if copied != size {
                return Err(ImageError::Malformed(format!(
                    "{child} is {size} bytes but gave {copied}"
                )));
            }
            set.insert(child_rel, Extracted { path: target, size });
        } else {
            return Err(ImageError::Malformed(format!(
                "{child} is a link or a device, not a file: a slot's boot files must be plain"
            )));
        }
    }
    Ok(())
}
