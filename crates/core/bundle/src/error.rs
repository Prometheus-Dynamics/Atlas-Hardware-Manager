use thiserror::Error;

/// Why a slot bundle could not be made, or a bundle is not one.
#[derive(Debug, Error)]
pub enum BundleError {
    #[error("{0}")]
    Io(String),
    /// A first-flash image could not be read; the reader's own message is kept.
    #[error("{0}")]
    Image(#[from] atlas_image::ImageError),
    #[error("slot id {0:?} must be 1 to 64 of A-Z, a-z, 0-9, '.', '_' or '-'")]
    InvalidId(String),
    #[error("{field} {value:?} must be 1 to 64 of a-z, 0-9 or '-'")]
    InvalidName { field: &'static str, value: String },
    #[error("version {0:?} must be 1 to 128 printable characters")]
    InvalidVersion(String),
    #[error("boot file {0:?} must be a relative path of plain names under boot/")]
    InvalidBootPath(String),
    #[error("boot file {0:?} is listed twice")]
    DuplicateBootFile(String),
    #[error("the boot files have no cmdline.txt")]
    MissingCmdline,
    /// The verity values do not describe the slot file: a value does not fit the
    /// layout, or the file is not its data and whole tree (see `atlas-image`).
    #[error(transparent)]
    Verity(#[from] atlas_image::VerityError),
    /// The data, salted and hashed, does not give the manifest's root hash: a wrong
    /// salt, or data that is not the slot's.
    #[error(
        "the verity root hash of the data is {actual}, but verity.root_hash is {expected}: the data does not match its root hash, or the salt is wrong"
    )]
    RootHashMismatch { expected: String, actual: String },
    #[error(
        "the slot file is {actual}, its manifest says {expected}: the image does not match its manifest"
    )]
    ImageDigest { expected: String, actual: String },
    #[error(
        "this slot is for board {bundle:?}, not the target board {target:?}: refused before anything is pushed"
    )]
    BoardMismatch { bundle: String, target: String },
    #[error("{member} changed between reads: it was {expected}, now {actual}")]
    Changed {
        member: String,
        expected: String,
        actual: String,
    },
    #[error("the bundle is not valid: {0}")]
    Malformed(String),
    /// The signature is by a key the list holds, and it does not verify the slot's message.
    #[error(
        "slot.sig is signed by a key this board holds, and it does not verify slot.json: refused"
    )]
    BadSignature,
    /// `boot/cmdline.txt` lacks the word for a boot source the board states.
    #[error(
        "boot/cmdline.txt has no {word:?} for boot source {boot_source}: the builder writes the whole command line, and the installer appends nothing"
    )]
    MissingBootSource { boot_source: String, word: String },
}

impl BundleError {
    pub(crate) fn io(what: &str, error: std::io::Error) -> Self {
        Self::Io(format!("reading {what}: {error}"))
    }
}
