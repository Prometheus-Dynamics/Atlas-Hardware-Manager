//! The Rockchip boot-ROM driver. It finds the devices in a Rockchip USB boot
//! ROM (maskrom or loader mode) on the USB ids it is given, says who each one
//! is, and grants `Flash` in the boot-ROM state. A device in that state takes
//! recovery work only.
//!
//! The driver runs no loader. Loading the boot ROM is `atlas-rockchip`'s flow,
//! called by the flash job with the device's location. The ids are data from
//! the caller, so no board or chip-number table lives here.
//!
//! Each identity says which mode the device is in, in its `rom-mode`
//! attribute: maskrom or loader, read from the device's `bcdUSB`
//! ([`atlas_rockchip::Mode`]). A caller that knows better sets the mode with
//! [`RockchipDriver::with_mode`], and the override applies to every device
//! the driver reports.
//!
//! This crate depends on no other driver. It is named `atlas-driver-rockchip`
//! because `atlas-rockchip` is the flash crate in `crates/flash/rockchip`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_rockchip::Mode;
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkKind,
};
use atlas_usb::{DeviceSummary, UsbBus, UsbId};

/// The family every Rockchip boot ROM belongs to. It is the family id
/// `rockchip-maskrom` in the device descriptions.
pub const FAMILY: &str = "rockchip-maskrom";

pub struct RockchipDriver {
    manifest: DriverManifest,
    bus: Arc<dyn UsbBus>,
    ids: Vec<UsbId>,
    mode_override: Option<Mode>,
}

impl RockchipDriver {
    /// A driver that finds devices with any of `ids` on `bus`.
    pub fn new(bus: Arc<dyn UsbBus>, ids: Vec<UsbId>) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Rockchip boot ROM".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbBoot],
                priority: 0,
            },
            bus,
            ids,
            mode_override: None,
        }
    }

    /// Reports every device in `mode`, instead of the mode its `bcdUSB` says.
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode_override = Some(mode);
        self
    }

    /// The mode a device is in: the override, else its `bcdUSB`.
    fn mode_of(&self, device: &DeviceSummary) -> Mode {
        self.mode_override.unwrap_or_else(|| Mode::of(device))
    }

    fn is_boot_rom(&self, device: &DeviceSummary) -> bool {
        self.ids.iter().any(|id| id.matches(device))
    }
}

impl fmt::Debug for RockchipDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RockchipDriver")
            .field("manifest", &self.manifest)
            .field("ids", &self.ids)
            .finish_non_exhaustive()
    }
}

/// The stable key: the serial when the device reports one, else its USB port.
/// A port can change if the device is moved, which is the best a boot ROM
/// without a serial offers.
fn key_for(device: &DeviceSummary) -> Result<DeviceKey, atlas_types::KeyError> {
    let serial = device
        .serial
        .clone()
        .unwrap_or_else(|| format!("usb-{}", device.location));
    DeviceKey::board(FAMILY, serial)
}

#[async_trait]
impl Driver for RockchipDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if link.kind != LinkKind::UsbBoot {
            return Ok(Vec::new());
        }
        let devices = self
            .bus
            .devices()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        Ok(devices
            .iter()
            .filter(|device| self.is_boot_rom(device))
            .map(|device| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: device.location.clone(),
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let devices = self
            .bus
            .devices()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        let device = devices
            .iter()
            .find(|device| device.location == candidate.address && self.is_boot_rom(device))
            .ok_or_else(|| {
                DriverError::Unreachable(format!(
                    "no boot-ROM device at {}; it may have been unplugged or has re-enumerated",
                    candidate.address
                ))
            })?;
        Ok(Identity {
            key: key_for(device).map_err(|error| DriverError::Incompatible(error.reason))?,
            model: format!(
                "Rockchip boot ROM ({:04x}:{:04x})",
                device.vendor, device.product
            ),
            state: DeviceState::BootRom,
            lemnos: atlas_types::LemnosState::Absent,
            versions: BTreeMap::new(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: BTreeMap::from([
                ("usb-port".to_string(), device.location.clone()),
                (
                    "usb-id".to_string(),
                    format!("{:04x}:{:04x}", device.vendor, device.product),
                ),
                (
                    "rom-mode".to_string(),
                    self.mode_of(device).label().to_string(),
                ),
            ]),
        })
    }

    fn capabilities(&self, device: &Identity) -> CapabilitySet {
        if device.state == DeviceState::BootRom {
            CapabilitySet::from_kinds([CapabilityKind::Flash])
        } else {
            CapabilitySet::new()
        }
    }
}
