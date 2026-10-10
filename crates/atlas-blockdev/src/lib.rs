//! Disks for Atlas: finding them, deciding which are safe to overwrite, and
//! writing an image with read-back verification, on Linux, Windows, and
//! macOS.
//!
//! Raw writes need elevated rights, so the app never writes itself. It
//! launches `atlas-helper` elevated ([`HelperClient`]), and the helper uses
//! [`write_image`] after re-checking the target with [`list_disks`]. The
//! helper refuses any disk that holds the running system, is not removable
//! or USB, or changed size since the app chose it.

mod client;
mod disk;
mod error;
mod image;
mod messages;
mod platform;
mod session;
mod writer;

pub use client::{HelperClient, HelperRequest};
pub use disk::{Disk, check_target};
pub use error::BlockError;
pub use image::{ImageFormat, detect_format};
pub use messages::HelperMessage;
pub use platform::list_disks;
pub use session::{
    eject, mount_read_only, open_folder, read_boot_file, unmount_all, write_boot_file,
};
pub use writer::{WriteOptions, WriteProgress, WriteReport, write_image};
