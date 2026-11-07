//! UBI on raw NAND, for the boards that boot from SPI-NAND (the K230D
//! boards, Flame and the Luckfox Pico Pro on RV1106, Rockchip L variants):
//!
//! - [`read_image`] (Gaia's `gaia_formats::ubi`): reads and checks the images
//!   Gaia writes for them (its G8, `[[image.assembly.ubis]]`, the bytes
//!   `ubinize` writes);
//! - [`format`]: writes one to a raw flash as `ubiformat -f` does, through
//!   any [`format::RawNand`] (the Rockchip loader, the K230 burner);
//! - [`slots`] and [`volume`]: on a running board, writes a slot image into
//!   the inactive one of the `rootfs_a`/`rootfs_b` volumes and reads it back
//!   (the `atlas-ubi` binary a NAND board's device package carries);
//! - [`nand_slots`] and [`mtd`]: Gaia's slot environments on such a board, the
//!   trial and the commit in the order that keeps it bootable.
//!
//! The on-media formats (headers, CRCs, the image reader) are Gaia's, in the
//! `gaia-formats` crate; [`headers`] keeps the names some sibling crates use.
//! Written from the kernel's UBI format and Gaia's writer; checked against
//! images built the way Gaia builds them. Untested on a board.

pub mod bundle;
pub use gaia_formats::crc;
pub mod format;
pub mod headers;
pub mod mtd;
pub mod nand_slots;
pub mod slots;
pub mod volume;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod sys;

#[cfg(test)]
mod test_images;

pub use format::{FormatError, FormatReport, RawNand, format};
pub use gaia_formats::ubi::{Image, Volume, find_peb_size, read_image};
