use std::fmt;

use atlas_usb::UsbError;

/// What went wrong talking to a fastboot device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FastbootError {
    /// The USB link failed: a stall, a timeout, or the device left the bus.
    /// A stall or a timeout is worth a retry: reopen the device and start
    /// again.
    Usb(UsbError),
    /// The device refused a command with `FAIL`.
    Refused {
        command: &'static str,
        message: String,
    },
    /// The device answered, but not as the protocol says.
    Protocol(String),
    /// There is nothing to send.
    EmptyImage,
    /// The image is larger than the device takes in one download.
    ImageTooLarge { size: u64, max: u64 },
    /// `boot` was called before an image was downloaded.
    NotStaged,
}

impl fmt::Display for FastbootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb(error) => write!(f, "fastboot: {error}"),
            Self::Refused { command, message } => {
                write!(f, "the device refused {command}: {message}")
            }
            Self::Protocol(message) => write!(f, "fastboot protocol error: {message}"),
            Self::EmptyImage => f.write_str("the image is empty"),
            Self::ImageTooLarge { size, max } => write!(
                f,
                "the image is {size} bytes, but the device takes at most {max} bytes per download"
            ),
            Self::NotStaged => f.write_str("nothing was downloaded: download an image before boot"),
        }
    }
}

impl std::error::Error for FastbootError {}

impl From<UsbError> for FastbootError {
    fn from(error: UsbError) -> Self {
        Self::Usb(error)
    }
}
