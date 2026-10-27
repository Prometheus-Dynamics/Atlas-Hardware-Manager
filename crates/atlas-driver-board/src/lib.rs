//! Running boards, whatever OS they run, through device contract v1:
//!
//! - mDNS service `_pd-device._tcp` with TXT `contract model rev serial os
//!   os_ver path`. The SRV port is authoritative.
//! - `GET <path>` (default `/.well-known/pd-device`) returning the identity
//!   JSON: model, revision, serial, hostname, OS, device package, bootloader,
//!   update methods, management URL, MACs.
//! - Optionally, `endpoints` for metrics, logs, and actions, and a camera
//!   stream. Atlas offers each only when the device lists it; see `live`.
//! - `ab-tryboot` in `update_methods`: A/B updates over SSH; see `ssh`.
//! - `slots-tryboot`: a board with dynamic slots, managed over SSH; see
//!   `slots`.
//! - `endpoints.status` and `endpoints.events`: what the board is doing,
//!   its health and drift, and its event log; see `status`.
//! - `usb_route` active, reached over USB: the board is the way onto the
//!   robot network, and what is on it (boards, the robot controller, the
//!   radio) are its children; see `BoardDriver::children`, `sweep` and
//!   docs/robot-network.md.
//!
//! The device family is the hardware model from its device package
//! (`raze`); the OS is an attribute. One [`BoardDriver`] serves each model in
//! the device catalog and they share one mDNS browser.

mod browse;
pub mod camera;
mod console;
mod contract;
mod driver;
mod gadget;
mod live;
mod serial;
mod slots;
mod ssh;
mod ssh_actions;
mod status;
mod stream;
mod sweep;
pub mod vision;

use contract::IDENTITY_PATH;
pub use driver::{CatalogHealth, NetworkLinks, drivers_for_catalog};
pub use gadget::UsbGadgetLinks;
pub use slots::set_bundle_host;
pub use ssh::{SshAccess, SshConfig, host_keys, private_key_for};
