//! The journal of a write: how far the disk is proven to hold the image.
//! It lets a later run skip what is already there. Skipped bytes are still
//! read and compared, so a stale journal costs time, never correctness.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Journal {
    /// Which image: its SHA-256 when known, else its size and modification time.
    pub(crate) image: String,
    /// Which target, from [`Target::identity`](crate::Target::identity).
    pub(crate) target: String,
    pub(crate) segment_bytes: u64,
    /// Bytes from the start of the disk that held the image when last saved.
    pub(crate) proven: u64,
}

impl Journal {
    pub(crate) fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    /// Replaces the file whole, so a crash leaves the old or the new one. The
    /// temporary file is made new, never reused, so a link planted at its
    /// name is not followed.
    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        let partial = partial_path(path);
        let _ = fs::remove_file(&partial);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, path)
    }

    pub(crate) fn remove(path: &Path) {
        let _ = fs::remove_file(path);
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

/// The journal for `image` when none is given: beside it, `<image>.journal`.
pub fn journal_path(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_owned();
    name.push(".journal");
    PathBuf::from(name)
}

/// Names the image a journal belongs to: its hash when known, else its size
/// and modification time.
pub(crate) fn image_identity(image: &Path, sha256: Option<&str>) -> io::Result<String> {
    if let Some(sha) = sha256 {
        return Ok(format!("sha256:{}", sha.trim().to_ascii_lowercase()));
    }
    let meta = fs::metadata(image)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    Ok(format!("{}:{modified}", meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_saves_and_loads_whole() {
        let dir = std::env::temp_dir().join(format!("atlas-disk-journal-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("image.img.xz.journal");
        let journal = Journal {
            image: "sha256:ab".into(),
            target: "1-4.2".into(),
            segment_bytes: 4096,
            proven: 8192,
        };
        journal.save(&path).unwrap();
        assert_eq!(Journal::load(&path), Some(journal));
        assert!(!partial_path(&path).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_journal_names_its_image_by_hash_when_known() {
        let image = std::env::temp_dir().join("atlas-disk-identity-unused");
        assert_eq!(image_identity(&image, Some(" ABC ")).unwrap(), "sha256:abc");
    }
}
