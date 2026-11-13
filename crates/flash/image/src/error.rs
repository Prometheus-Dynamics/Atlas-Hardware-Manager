use thiserror::Error;

/// Why a first-flash image could not be read. Every malformed input ends in one
/// of these; nothing in the reader panics on bytes it did not write itself.
#[derive(Debug, Error)]
pub enum ImageError {
    /// The file could not be opened, read or written (the scratch spool too).
    #[error("{0}")]
    Io(String),
    /// The partition table is not a GPT this reader accepts.
    #[error("the partition table is not valid: {0}")]
    Table(String),
    /// The partitions are not a form this reader knows, or are out of order.
    #[error("the image layout: {0}")]
    Layout(String),
    /// A file, directory or partition the reader needs is not in the image.
    #[error("{0}")]
    Missing(String),
    /// The image ends before the data the reader needs.
    #[error("the image is truncated: {0}")]
    Truncated(String),
    /// The xz stream is damaged or uses a feature this decoder does not have.
    #[error("the image does not decode: {0}")]
    Decode(String),
    /// The ext4 or FAT structures are not readable.
    #[error("{0}")]
    Filesystem(String),
    /// A file in the image is not what a slot's file must be (a link, a device,
    /// a name that is not UTF-8, a size that changed).
    #[error("{0}")]
    Malformed(String),
    /// A size cap was hit (a partition, a boot file, the boot files in total).
    #[error("{0}")]
    Limit(String),
    /// A slot id or a path inside the image is not a plain name.
    #[error("{0}")]
    InvalidName(String),
}

impl ImageError {
    pub(crate) fn io(what: &str, error: &std::io::Error) -> Self {
        Self::Io(format!("{what}: {error}"))
    }
}

impl From<atlas_xzdec::XzError> for ImageError {
    fn from(error: atlas_xzdec::XzError) -> Self {
        match error {
            atlas_xzdec::XzError::Io(text) => ImageError::Io(text),
            atlas_xzdec::XzError::Truncated(text) => ImageError::Truncated(text),
            atlas_xzdec::XzError::Malformed(text) => ImageError::Malformed(format!("not a first-flash image: {text}")),
            atlas_xzdec::XzError::Limit(text) => ImageError::Limit(text),
        }
    }
}
