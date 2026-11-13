//! The seam between a first-flash image and the bundle writer.
//!
//! The writer needs four things from the image: the slot's four verity values
//! (`verity.root_hash`, `verity.salt`, `verity.data_blocks`, `verity.hash_offset`),
//! the slot's EROFS file with its hash tree appended, and the boot files that go
//! with it. Each stream is read from
//! its start every time it is asked for: the writer reads the rootfs and
//! the boot files twice (once to hash them into `slot.json`, once to write
//! them into the tar), so a source must give the same bytes both times.
//!
//! The real source is [`crate::ImageSource`], over `atlas-image`'s slot.

use std::io::Read;

use atlas_image::Verity;

use crate::BundleError;

/// A boot file of the slot: its path under `boot/` and its size in bytes.
/// The size is informational; the writer hashes what the stream gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootFile {
    pub path: String,
    pub size: u64,
}

/// One slot of a first-flash image.
pub trait SlotSource {
    /// The slot's four verity values. The writer checks their layout and that the
    /// data gives the root hash; it does not trust the root because it is given.
    fn verity(&self) -> Result<Verity, BundleError>;

    /// The slot's file, read from its first byte: the data (`data_blocks` blocks),
    /// then the dm-verity tree appended.
    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError>;

    /// The boot files of the slot (kernel, device trees, initramfs,
    /// `cmdline.txt`, ...), by path under `boot/`.
    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError>;

    /// One boot file, read from its first byte.
    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError>;
}
