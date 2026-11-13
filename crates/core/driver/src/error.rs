use thiserror::Error;

/// Errors a driver reports. Messages are shown to users, so they say what
/// happened and, where possible, what to try next.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DriverError {
    #[error("device did not respond: {0}")]
    Unreachable(String),
    /// The device cannot do this as things are. The message says why and what
    /// would change it, so it stands alone.
    #[error("{0}")]
    Incompatible(String),
    /// This way to the device cannot do it; the message stands alone.
    #[error("{0}")]
    Unsupported(String),
    #[error("cancelled before any change was made")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}
