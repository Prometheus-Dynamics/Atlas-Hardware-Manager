//! The FAT side of the real form's boot partition, read with `fatfs` (MIT). The
//! partition is a spool file opened read-only: no write happens, and a write
//! attempt fails instead of changing the spool.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use fatfs::{Dir, FileSystem, FsOptions};

use crate::budget::Budget;
use crate::error::ImageError;
use crate::names;
use crate::route::{BootSet, Extracted};

const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: u32 = 8;

fn fat_error(error: io::Error) -> ImageError {
    ImageError::Filesystem(format!("FAT: {error}"))
}

/// Copies the tree under `dir` (a relative path, such as `slots/alpha`) of the
/// FAT partition at `spool` into `dest` and returns what it holds.
pub(crate) fn extract(
    spool: &Path,
    dir: &str,
    dest: &Path,
    budget: &mut Budget,
) -> Result<BootSet, ImageError> {
    let file = File::open(spool).map_err(|error| ImageError::io("the boot partition", &error))?;
    let fs = FileSystem::new(file, FsOptions::new()).map_err(fat_error)?;
    let mut here = fs.root_dir();
    for part in names::split_rel(dir)? {
        here = here
            .open_dir(part)
            .map_err(|_| ImageError::Missing(format!("the boot partition has no {dir}")))?;
    }
    let mut set = BootSet::new();
    walk(&here, "", 0, dest, budget, &mut set)?;
    Ok(set)
}

fn walk(
    dir: &Dir<'_, File>,
    rel: &str,
    depth: u32,
    dest: &Path,
    budget: &mut Budget,
    set: &mut BootSet,
) -> Result<(), ImageError> {
    if depth > MAX_DEPTH {
        return Err(ImageError::Limit(format!(
            "the boot tree nests deeper than {MAX_DEPTH}"
        )));
    }
    let mut entries = Vec::new();
    for entry in dir.iter() {
        if entries.len() >= MAX_ENTRIES {
            return Err(ImageError::Limit(format!(
                "a boot directory holds more than {MAX_ENTRIES} entries"
            )));
        }
        let entry = entry.map_err(fat_error)?;
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        names::check_component(&name)?;
        entries.push((name, entry.is_dir(), entry.len()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, is_dir, size) in entries {
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if is_dir {
            let sub = dir.open_dir(&name).map_err(fat_error)?;
            walk(&sub, &child_rel, depth + 1, dest, budget, set)?;
            continue;
        }
        budget.take(size)?;
        let target = dest.join(&child_rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| ImageError::io("the boot files", &error))?;
        }
        let mut source = dir.open_file(&name).map_err(fat_error)?;
        let mut out =
            File::create(&target).map_err(|error| ImageError::io("the boot files", &error))?;
        let copied = io::copy(&mut (&mut source).take(size), &mut out)
            .map_err(|error| ImageError::io("the boot files", &error))?;
        if copied != size {
            return Err(ImageError::Malformed(format!(
                "{child_rel} is {size} bytes but gave {copied}"
            )));
        }
        set.insert(child_rel, Extracted { path: target, size });
    }
    Ok(())
}
