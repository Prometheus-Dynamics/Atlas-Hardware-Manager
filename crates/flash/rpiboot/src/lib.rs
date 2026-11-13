//! Raspberry Pi USB boot. A Pi in its boot ROM takes two rounds on the same
//! port:
//!
//! 1. The boot ROM accepts a second-stage bootloader (`bootcode4.bin`,
//!    `bootcode5.bin`) over vendor requests and bulk writes, then resets.
//! 2. The device re-enumerates at the same port and runs a file server: it
//!    asks the host for files by name (`config.txt`, `boot.img`, ...) until it
//!    says it is done, or it leaves the bus because it booted what it got.
//!
//! [`boot_device`] runs both rounds. The caller passes the boot files
//! ([`BootFiles`]: a folder, or an in-memory set) and the device's location.
//! This crate ships no files and knows no board: which USB id is which SoC is
//! data the caller passes in ([`Chip`]).
//!
//! Loaders see only `atlas-usb`'s [`UsbBus`](atlas_usb::UsbBus) and
//! [`UsbIo`](atlas_usb::UsbIo), so every round runs against the simulated bus
//! in tests.
//!
//! The EEPROM's `bootconf.txt` is read and written by [`Bootconf`].

mod boot;
mod bootconf;
mod bootfiles;
mod chip;
mod error;
mod protocol;

pub use atlas_usb::{OpenOptions, Timeouts};
pub use boot::{BootSettings, boot_device};
pub use bootconf::Bootconf;
pub use bootfiles::BootFiles;
pub use chip::Chip;
pub use error::RpiBootError;
pub use protocol::{BootEvent, FileServerOutcome, file_server, second_stage};
