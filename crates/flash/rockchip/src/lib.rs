//! Rockchip boot ROMs over USB, in Rust, with no vendor tool: the loader
//! file, the maskrom download that runs its DDR init and usbplug, and the
//! read-only rockusb queries the usbplug answers once it is up.
//!
//! The protocol follows rkdeveloptool (RKComm, RKDevice, RKBoot.h) and
//! rockchip's U-Boot rockusb. Everything goes through the traits of
//! [`atlas_usb`], so the same code runs on a real bus and on
//! [`sim::SimRockchip`], the simulated board the tests use. Nothing here
//! names a board or a product: a caller passes the USB ids it wants and the
//! chip number it expects, both as data.
//!
//! Not here: writing storage, erasing, changing storage, and starting an
//! image from RAM. Dev's write path is the first of these; the last is not
//! shown by dev's code or docs. See the PR text for what is proven and what
//! is not.

pub mod crc;
pub mod flow;
pub mod loader;
pub mod maskrom;
pub mod rockusb;
pub mod sim;

/// What the USB layer reports: [`atlas_usb::UsbError`] for every transfer.
pub type UsbResult<T> = Result<T, atlas_usb::UsbError>;

pub use flow::{BoardReport, chip_agrees, ready, report, run_usbplug};
pub use loader::{Loader, LoaderError};
