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
//! - `endpoints.status` and `endpoints.events`: what the board is doing,
//!   its health and drift, and its event log; see `status`.
//!
//! The device family is the hardware model from its device package
//! (`raze`); the OS is an attribute. One [`BoardDriver`] serves each model in
//! the device catalog and they share one mDNS browser.

mod browse;
mod contract;
mod driver;
mod gadget;
mod live;
mod serial;
mod ssh;
mod ssh_actions;
mod status;
mod stream;

pub use browse::{Browser, SERVICE_TYPE};
pub use contract::{BoardIdentity, CLOCK_TOLERANCE_S, IDENTITY_PATH};
pub use driver::{BoardDriver, NetworkLinks, drivers_for_catalog};
pub use gadget::UsbGadgetLinks;
pub use serial::serial_console;
pub use ssh::{AB_METHOD, SshAccess, SshConfig, private_key_for};
pub use ssh_actions::{
    POWER_OFF_ACTION, REBOOT_ACTION, SELFTEST_DIAGNOSTIC, SET_CLOCK_ACTION, UPDATE_CANCEL_ACTION,
    UPDATE_ROLLBACK_ACTION, USB_BOOT_ACTION, USB_BOOT_METHOD,
};
