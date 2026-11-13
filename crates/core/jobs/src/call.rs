use std::fmt;
use std::future::Future;
use std::time::Duration;

use atlas_driver::DriverError;

/// Why a driver call did not give a result.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The driver answered with an error.
    Driver(DriverError),
    /// No answer within the limit. The call was dropped.
    TimedOut(Duration),
    /// The call panicked. Contained to this call; the caller carries on.
    Panicked(String),
}

impl CallError {
    pub(crate) fn timed_out(&self) -> bool {
        matches!(self, Self::TimedOut(_))
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Driver(error) => write!(f, "{error}"),
            Self::TimedOut(limit) => write!(f, "no answer within {} s", limit.as_secs_f64()),
            Self::Panicked(message) => write!(f, "driver call panicked: {message}"),
        }
    }
}

/// Runs one driver call in its own task with its own timeout. A slow call
/// only holds its own task; a panic is reported as an error, not propagated.
pub(crate) async fn guarded<T, F>(limit: Duration, call: F) -> Result<T, CallError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, DriverError>> + Send + 'static,
{
    match tokio::spawn(async move { tokio::time::timeout(limit, call).await }).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => Err(CallError::Driver(error)),
        Ok(Err(_elapsed)) => Err(CallError::TimedOut(limit)),
        Err(join) => Err(CallError::Panicked(join.to_string())),
    }
}
