use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlockError {
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("could not list disks: {0}")]
    List(String),
    #[error("{0}")]
    Unsafe(String),
    #[error("no disk at {0}; it may have been unplugged")]
    NotFound(String),
    #[error("{0} is in use and could not be unmounted: {1}")]
    Busy(String, String),
    #[error("the image could not be read: {0}")]
    Image(String),
    #[error("the image does not fit: {image} bytes onto a {disk} byte disk")]
    TooLarge { image: u64, disk: u64 },
    #[error(
        "verification failed: the disk does not match what was written (expected {expected}, read {actual})"
    )]
    VerifyMismatch { expected: String, actual: String },
    #[error("the image failed its SHA-256 check (expected {expected}, got {actual})")]
    ImageHashMismatch { expected: String, actual: String },
    #[error("the write was cancelled; the disk is partly written and needs a new write")]
    Cancelled,
    #[error("{message}")]
    Elevation { message: String, fix: String },
    #[error("the helper failed: {0}")]
    Helper(String),
}

impl BlockError {
    pub(crate) fn io(path: impl Into<PathBuf>, error: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }
}
