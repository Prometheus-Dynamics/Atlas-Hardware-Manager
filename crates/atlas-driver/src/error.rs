use thiserror::Error;

/// Errors a driver reports. Messages are shown to users, so they say what
/// happened and, where possible, what to try next.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DriverError {
    #[error("device did not respond: {0}")]
    Unreachable(String),
    /// The device can't do this as things are: the message says why and
    /// what would change it (it's often not the release at all: a refused
    /// SSH key, a busy board, no room), so it stands alone.
    #[error("{0}")]
    Incompatible(String),
    /// This way to the device can't do it; the message stands alone.
    #[error("{0}")]
    Unsupported(String),
    #[error("cancelled before any change was made")]
    Cancelled,
    #[error("{step} failed: {message}")]
    StepFailed { step: String, message: String },
    #[error("{0}")]
    Other(String),
}
