//! Disks and cards for Atlas: finding them, deciding which may be written,
//! and writing an image to one with read-back verification, through drops.
//!
//! The write pipeline ([`write_disk`]) is one piece of code on every system.
//! What differs per system sits behind [`Platform`]: listing disks, taking
//! their volumes off, opening the raw device, finishing it, and who may do it.
//! Linux is proven on hardware. Windows and macOS are implemented but unproven:
//! a write to a real disk there is refused until the host test has passed and
//! the proof is set in one place (`platform.rs`).
//!
//! Nothing here names a board, a product or a device family, and nothing here
//! reads Atlas types. Writing a disk needs elevated rights, so only the helper
//! (`atlas-helper`) calls [`write_disk`] on a real disk. Everything else in this
//! crate runs on files and on made-up disks.

mod device;
mod disk;
mod error;
mod image;
mod journal;
mod platform;
// Whole-sector I/O is what Windows and macOS raw devices need; Linux writes
// through the page cache and only its tests use it.
#[cfg_attr(target_os = "linux", allow(dead_code))]
mod sector;
mod target;
mod writer;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

pub use device::{Device, WriteRequest, write_disk};
pub use disk::{
    Disk, DiskSource, LARGE_DISK_BYTES, MAX_CARD_BYTES, check_size_choice, check_target,
    device_path, find_again, is_system_mount, same_disk,
};
pub use error::DiskError;
pub use image::{ImageFormat, detect_format};
pub use journal::journal_path;
pub use platform::{
    Acknowledgement, Guard, LINUX_PROOF, MACOS_PROOF, Platform, Proof, RawDisk, SysfsDisks,
    WINDOWS_PROOF, host_acknowledgement, host_platform, host_proof, list_disks,
};
pub use target::Target;
pub use writer::{ResumePolicy, WriteOptions, WriteProgress, WriteReport, write_image};

#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform;
#[cfg(target_os = "macos")]
pub use macos::MacPlatform;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub use unsupported::UnsupportedPlatform;
#[cfg(windows)]
pub use windows::WindowsPlatform;
