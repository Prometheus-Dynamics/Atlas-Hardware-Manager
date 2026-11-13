use std::fmt;

use atlas_usb::UsbError;

/// What went wrong in a USB boot. Messages are shown to people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpiBootError {
    /// The USB link failed; the inner error says how and what to do.
    Usb(UsbError),
    /// The boot ROM refused the second stage, with its error code.
    RomRefused(i32),
    /// The device answered, but not as the protocol says.
    Protocol(String),
    /// The boot files have no file the device asked for or needs.
    MissingFile { name: String, chip: &'static str },
    /// The boot files folder cannot be read.
    Files(String),
    /// A bootconf line or setting is not valid.
    Bootconf(String),
    /// The caller cancelled before the boot finished.
    Cancelled,
}

impl fmt::Display for RpiBootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb(error) => write!(f, "{error}"),
            Self::RomRefused(code) => {
                write!(
                    f,
                    "the boot ROM refused the second stage (error 0x{code:x})"
                )
            }
            Self::Protocol(message) => write!(f, "the USB boot failed: {message}"),
            Self::MissingFile { name, chip } => {
                write!(f, "the boot files have no {name} for {chip}")
            }
            Self::Files(message) => f.write_str(message),
            Self::Bootconf(message) => write!(f, "bootconf: {message}"),
            Self::Cancelled => f.write_str("the USB boot was cancelled"),
        }
    }
}

impl std::error::Error for RpiBootError {}

impl From<UsbError> for RpiBootError {
    fn from(error: UsbError) -> Self {
        Self::Usb(error)
    }
}
