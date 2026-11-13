//! Disks and cards for Atlas: finding them, deciding which may be written,
//! and writing an image to one with read-back verification, through drops.
//!
//! Nothing here names a board, a product or a device family, and nothing
//! here reads Atlas types. Writing a disk needs elevated rights, so only the
//! helper (`atlas-helper`) calls [`write_image`] on a real disk. Everything
//! else in this crate runs on files and on made-up disks.

mod disk;
mod error;
mod image;
mod journal;
mod target;
mod writer;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

pub use disk::{
    Disk, DiskSource, MAX_CARD_BYTES, check_target, device_path, find_again, is_system_mount,
};
pub use error::DiskError;
pub use image::{ImageFormat, detect_format};
pub use journal::journal_path;
pub use target::Target;
pub use writer::{ResumePolicy, WriteOptions, WriteProgress, WriteReport, write_image};

#[cfg(target_os = "linux")]
pub use linux::{BlockDevice, SysfsDisks, list_disks};
#[cfg(not(target_os = "linux"))]
pub use unsupported::{BlockDevice, SysfsDisks, list_disks};
