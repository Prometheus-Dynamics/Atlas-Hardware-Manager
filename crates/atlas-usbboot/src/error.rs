use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsbBootError {
    #[error("boot files not found in {0}: expected bootfiles.bin or bootcode files")]
    BootFilesMissing(PathBuf),
    #[error("the boot files have no `{name}` for {chip}")]
    MissingFile { name: String, chip: &'static str },
    #[error("could not read {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("bootfiles.bin is damaged: {0}")]
    BadArchive(String),
    #[error("no Pi in USB boot mode appeared within {seconds} s")]
    Timeout { seconds: u64 },
    #[error("the Pi disconnected during USB boot")]
    Disconnected,
    #[error("{message}")]
    Access { message: String, fix: String },
    #[error("USB error: {0}")]
    Usb(String),
    #[error("the Pi rejected the boot: {0}")]
    Protocol(String),
    #[error("USB boot was cancelled")]
    Cancelled,
}

impl UsbBootError {
    /// What the user can do about this error, when there is something.
    pub fn fix(&self) -> Option<&str> {
        match self {
            Self::Access { fix, .. } => Some(fix),
            Self::Timeout { .. } => Some(
                "Fit the nRPIBOOT jumper (or hold the boot button), connect the USB cable to the \
                 device's USB boot port, then power it on.",
            ),
            _ => None,
        }
    }
}
