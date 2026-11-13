use thiserror::Error;

/// Why a slot bundle could not be made, or a bundle is not one.
#[derive(Debug, Error)]
pub enum BundleError {
    #[error("{0}")]
    Io(String),
    #[error("slot id {0:?} must be 1 to 64 of A-Z, a-z, 0-9, '.', '_' or '-'")]
    InvalidId(String),
    #[error("boot file {0:?} must be a relative path of plain names under boot/")]
    InvalidBootPath(String),
    #[error("boot file {0:?} is listed twice")]
    DuplicateBootFile(String),
    #[error("the boot files have no cmdline.txt")]
    MissingCmdline,
    #[error("the verity table is not one this writer reads: {0}")]
    Table(String),
    #[error(
        "the verity root hash is {actual}, the table says {expected}: the image's tree does not match its table"
    )]
    RootHashMismatch { expected: String, actual: String },
    #[error("{member} changed between reads: it was {expected}, now {actual}")]
    Changed {
        member: String,
        expected: String,
        actual: String,
    },
    #[error("the bundle is not valid: {0}")]
    Malformed(String),
    #[error("slot.sig does not verify against slot.json with any trusted key")]
    BadSignature,
}

impl BundleError {
    pub(crate) fn io(what: &str, error: std::io::Error) -> Self {
        Self::Io(format!("reading {what}: {error}"))
    }
}
