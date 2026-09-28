use thiserror::Error;

/// Errors a driver reports. Messages are shown to users, so they say what
/// happened and, where possible, what to try next.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DriverError {
    #[error("device did not respond: {0}")]
    Unreachable(String),
    #[error("device is not compatible with this release: {0}")]
    Incompatible(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("cancelled before any change was made")]
    Cancelled,
    #[error("{step} failed: {message}")]
    StepFailed { step: String, message: String },
    #[error("{0}")]
    Other(String),
}
