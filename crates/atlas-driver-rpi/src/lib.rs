//! Raspberry Pi compute modules in USB boot mode.
//!
//! A board with its nRPIBOOT jumper fitted (or a blank eMMC) appears as a
//! recovery-mode device of family `rpi`. Recovering it:
//!
//! 1. **Transfer**: boots it over USB with the mass-storage-gadget files
//!    ([`atlas_usbboot`]), so its eMMC shows up as a USB disk.
//! 2. **Apply**: waits for that new disk and writes the chosen image with
//!    the elevated `atlas-helper` ([`atlas_blockdev`]).
//! 3. **Confirm**: reads the disk back and compares hashes.
//!
//! The device is not HeliOS-specific: any image can be written. After the
//! jumper comes off and it powers up, whatever OS was written takes over
//! (and its own driver finds it).

mod actions;
mod boot_files;
mod driver;
mod eeprom;
mod health;
mod recover;

pub use boot_files::find_boot_files;
pub use driver::{RPI_FAMILY, RpiConfig, RpiDriver, UsbBootLinks};
