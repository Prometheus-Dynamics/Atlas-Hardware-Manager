use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReleaseError {
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("{path} failed its SHA-256 check: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("{path} is {actual} bytes, the release says {expected}")]
    SizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    /// Security is on and the file or manifest has no good signature from a
    /// trusted, unexpired key.
    #[error("{name} is refused: {reason}")]
    Untrusted { name: String, reason: String },
    #[error("the manifest for {family} {version} has no valid signature from a trusted key")]
    BadSignature { family: String, version: String },
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid public key: {0}")]
    InvalidKey(String),
    #[error("{0} is not a first-flash image (.img.xz)")]
    NotAnImage(String),
    #[error("could not download {url}: {message}")]
    Download { url: String, message: String },
    #[error("no release with id `{0}`")]
    UnknownRelease(String),
    #[error("release `{0}` is not downloaded yet")]
    NotDownloaded(String),
    #[error("release catalog {path} is not valid: {message}")]
    Catalog { path: PathBuf, message: String },
}

impl ReleaseError {
    pub(crate) fn io(path: impl Into<PathBuf>, error: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }
}
