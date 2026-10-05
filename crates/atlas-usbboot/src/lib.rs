//! Raspberry Pi USB boot in pure Rust: a replacement for `rpiboot` that
//! works the same on Linux, Windows, and macOS with no bundled binary.
//!
//! A Pi in USB boot mode (nRPIBOOT jumper fitted, or an empty eMMC) shows
//! up as a Broadcom device (`0a5c:2711` for BCM2711/CM4, `0a5c:2712` for
//! BCM2712/CM5). Booting it takes two rounds on the same USB port:
//!
//! 1. The boot ROM accepts a second-stage bootloader (`bootcode4.bin` /
//!    `bootcode5.bin`) over a vendor control request and a bulk transfer.
//! 2. The device re-enumerates and runs a file server loop, asking the host
//!    for files by name (`config.txt`, `boot.img`, ...) until it is done.
//!
//! With the mass-storage-gadget boot files, the Pi then boots a tiny Linux
//! that exposes its eMMC as a USB disk, ready for an image write.
//!
//! This is a port of the protocol in `raspberrypi/usbboot` (`main.c`).

mod bootfiles;
mod chip;
mod error;
mod gadget;
mod hotplug;
mod protocol;
mod usb;

pub use bootfiles::BootFiles;
pub use chip::{BROADCOM_VENDOR_ID, Chip};
pub use error::UsbBootError;
pub use gadget::{STORAGE_GADGET_PRODUCT_ID, StorageGadget, list_storage_gadgets};
pub use hotplug::watch_usb;
pub use protocol::{BootEvent, BootTransport, FileServerOutcome, file_server, second_stage};
pub use usb::{
    BOOT_PRODUCT_IDS, BootDevice, BootOptions, BootOutcome, LINUX_UDEV_RULE, LINUX_UDEV_RULES_FILE,
    UsbBootTransport, boot_device, list_boot_devices,
};
