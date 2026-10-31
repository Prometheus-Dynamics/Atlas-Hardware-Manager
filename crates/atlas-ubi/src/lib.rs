//! UBI on raw NAND, for the boards that boot from SPI-NAND (the K230D
//! boards, Flame and the Luckfox Pico Pro on RV1106, Rockchip L variants):
//!
//! - [`image`]: reads and checks the images Gaia writes for them (its G8,
//!   `[[image.assembly.ubis]]`, the bytes `ubinize` writes);
//! - [`format`]: writes one to a raw flash as `ubiformat -f` does, through
//!   any [`format::RawNand`] (the Rockchip loader, the K230 burner);
//! - [`slots`] and [`volume`]: on a running board, writes a slot image into
//!   the inactive one of the `rootfs_a`/`rootfs_b` volumes and reads it back
//!   (the `atlas-ubi` binary a NAND board's device package carries).
//!
//! Written from the kernel's UBI format and Gaia's writer; checked against
//! images built the way Gaia builds them. Untested on a board.

pub mod crc;
pub mod format;
pub mod headers;
pub mod image;
pub mod slots;
pub mod volume;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod sys;

pub use format::{FormatReport, RawNand, format};
pub use image::{Geometry, Image, UbiError, Volume, find_peb_size, read_image};
