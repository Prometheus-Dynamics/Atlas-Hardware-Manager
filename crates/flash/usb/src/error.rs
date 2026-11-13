use std::fmt;

/// What went wrong on the USB link. Loaders match on the variant to decide
/// whether waiting or reopening can help.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsbError {
    /// The device left the bus: it reset, or it was unplugged. Handles opened
    /// before that stay dead; open again once it has re-enumerated.
    Disconnected,
    /// No device is at this location.
    NotFound(String),
    /// The device stalled the endpoint.
    Stall,
    /// A transfer got no answer within its timeout.
    Timeout(String),
    /// The device could not be opened or claimed, with the fix for the user.
    Access { message: String, fix: String },
    /// The device answered, but not as the protocol says.
    Protocol(String),
    /// Anything else the operating system reported.
    Other(String),
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("the device disconnected from USB"),
            Self::NotFound(location) => write!(f, "no USB device at {location}"),
            Self::Stall => f.write_str("the device stalled the transfer"),
            Self::Timeout(what) => write!(f, "{what} timed out"),
            Self::Access { message, fix } => write!(f, "{message} {fix}"),
            Self::Protocol(message) => write!(f, "the device answered unexpectedly: {message}"),
            Self::Other(message) => write!(f, "USB error: {message}"),
        }
    }
}

impl std::error::Error for UsbError {}
