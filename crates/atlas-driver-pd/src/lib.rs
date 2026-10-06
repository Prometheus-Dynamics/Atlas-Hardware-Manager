//! Running PD devices, whatever OS they run, through device contract v1:
//!
//! - mDNS service `_pd-device._tcp` with TXT `contract model rev serial os
//!   os_ver path`. The SRV port is authoritative.
//! - `GET <path>` (default `/.well-known/pd-device`) returning the identity
//!   JSON: model, revision, serial, hostname, OS, device package, bootloader,
//!   update methods, management URL, MACs.
//! - Optionally, `endpoints` for metrics, logs, and actions, and a camera
//!   stream. Atlas offers each only when the device lists it; see `live`.
//!
//! The device family is the hardware model from its device package
//! (`raze`); the OS is an attribute. One [`PdDriver`] serves each model in
//! the device catalog and they share one mDNS browser.

mod browse;
mod contract;
mod driver;
mod gadget;
mod live;
mod serial;

pub use browse::{Browser, SERVICE_TYPE};
pub use contract::{IDENTITY_PATH, PdIdentity};
pub use driver::{NetworkLinks, PdDriver, drivers_for_catalog};
pub use gadget::UsbGadgetLinks;
pub use serial::serial_console;
