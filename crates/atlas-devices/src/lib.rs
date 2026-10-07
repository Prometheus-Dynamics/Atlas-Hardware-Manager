//! Device packages: what Atlas knows about each hardware model.
//!
//! A device package lives in `devices/<model>/` in this repository and is
//! shared with every OS built for that hardware. Atlas reads two files:
//!
//! - `manifest.json`: model, revisions, identity rules, USB strings,
//!   recovery method and instructions, EEPROM files, capabilities, and
//!   hardware facts (LED ring, fan, I2C devices, self-test; `hardware`).
//! - `compat.json`: which OS versions are known to work. Informational
//!   only: Atlas warns, it never blocks.
//!
//! Parsing is lenient. Unknown fields are ignored so newer packages still
//! load, and only `contract` and `model` are required.

mod catalog;
mod compat;
mod gadget;
mod hardware;
mod manifest;

pub use catalog::{DeviceCatalog, DevicePackage, default_search_dirs};
pub use compat::{CompatEntry, CompatImage, CompatList, CompatStatus};
pub use gadget::{GadgetAddressing, GadgetSubnet, SERIAL_HASH_V1};
pub use hardware::{
    Fan, FanPwm, I2cAddress, I2cDevice, LedIndex, LedRing, SelfTestInfo, Verification,
};
pub use manifest::{
    DeviceManifest, EepromFile, EepromInfo, IdentityRules, Recovery, Revision, UsbBootId,
    UsbGadget, UsbId,
};

/// The device contract version this Atlas understands.
pub const CONTRACT: u32 = 1;
