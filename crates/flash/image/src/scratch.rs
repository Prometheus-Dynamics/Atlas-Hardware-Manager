//! The scratch directory: every spooled partition and every extracted boot file
//! lives in one directory made for this read. It is removed when the reader is
//! dropped, and also when a read fails part way, because the reader is dropped
//! on every exit path (including a panic's unwind).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ImageError;

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// Makes a new, empty directory under `parent`. It fails rather than reuse
    /// a directory that already exists.
    pub(crate) fn create(parent: &Path) -> Result<Self, ImageError> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let name = format!(
            "atlas-image-{}-{nanos}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = parent.join(name);
        std::fs::create_dir(&path)
            .map_err(|error| ImageError::io("the scratch directory", &error))?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Nothing useful can be done with a failure here; the directory is the
        // reader's own, so a leftover one is only disk space.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
