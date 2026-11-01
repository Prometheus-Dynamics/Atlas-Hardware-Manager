//! Boards Atlas flashes from their SoC's USB download mode, named by the
//! device catalog's board records (`recovery.method` `rockusb` or
//! `k230-usb`): a Rockchip board in maskrom or loader mode, a K230 board in
//! USB boot. Each shows as a device in recovery; its update writes an image
//! with the vendor's loader file and no vendor tool (atlas-socflash), reads
//! it back, and restarts the board.
//!
//! Written from the vendors' host tools; untested on a board (the records
//! say `"status": "untested"`, and Atlas says so before it flashes).

mod driver;
mod files;
mod flash;
mod layout;

pub use driver::{K230_FAMILY, ROCKCHIP_FAMILY, SocDriver, drivers};
pub use files::{find_loader, find_ubi_start, glob_match};
pub use layout::{Part, Planned, mtdparts, partition_set, uboot_env};
