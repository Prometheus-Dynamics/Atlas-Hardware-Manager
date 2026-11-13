//! The Raspberry Pi boot ROM driver. It finds the boards in a USB boot ROM on
//! a [`LinkKind::UsbBoot`] link, says what each one is, and grants `Flash` in
//! the boot ROM state. [`RpiDriver::boot`] runs the two USB boot rounds, which
//! leave the board running the flasher image. Writing the disk after that is
//! an ordinary update over the flasher's link, not this driver's job.
//!
//! What the driver knows about boards comes in as data: [`RpiConfig`] holds
//! the boot ROMs to recognise (their USB ids and SoCs, from the catalog) and
//! the flasher image's boot files. This crate names no board and no USB id.
//!
//! This crate depends on no other driver. Its USB access is the injected
//! [`UsbBus`], so tests run against the simulated bus.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest, HealthCheck};
use atlas_rpiboot::{
    BootEvent, BootFiles, BootSettings, Chip, FileServerOutcome, RpiBootError, boot_device,
};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkKind,
};
use atlas_usb::{DeviceSummary, UsbBus, UsbError, UsbId};

/// The family every boot ROM board belongs to.
pub const FAMILY: &str = "rpi";

/// A boot ROM this driver recognises, as catalog data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootRom {
    /// The USB id the boot ROM shows while it waits.
    pub usb: UsbId,
    /// Its SoC: the second stage and boot files it takes.
    pub chip: Chip,
    /// The model name the board is shown as.
    pub model: String,
}

/// What the driver is configured with.
#[derive(Clone, Debug)]
pub struct RpiConfig {
    /// The boot ROMs to recognise.
    pub boot_roms: Vec<BootRom>,
    /// How to reach a device in each round of the boot.
    pub settings: BootSettings,
    /// The flasher image's boot files. `None` means a board cannot be booted
    /// yet, and the health check says so.
    pub flasher: Option<Arc<BootFiles>>,
}

pub struct RpiDriver {
    manifest: DriverManifest,
    bus: Arc<dyn UsbBus>,
    config: RpiConfig,
}

impl fmt::Debug for RpiDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpiDriver")
            .field("manifest", &self.manifest)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl RpiDriver {
    /// A driver on `bus` with the boot ROMs and flasher image in `config`.
    pub fn new(bus: Arc<dyn UsbBus>, config: RpiConfig) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Raspberry Pi USB boot".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbBoot],
                priority: 0,
            },
            bus,
            config,
        }
    }

    fn rom_for(&self, device: &DeviceSummary) -> Option<&BootRom> {
        self.config
            .boot_roms
            .iter()
            .find(|rom| rom.usb.matches(device))
    }

    async fn device_at(&self, location: &str) -> Result<DeviceSummary, DriverError> {
        self.bus
            .devices()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?
            .into_iter()
            .find(|device| device.location == location)
            .ok_or_else(|| {
                DriverError::Unreachable(format!(
                    "no USB device at {location}; it may have been unplugged"
                ))
            })
    }

    /// Boots a board found by this driver into the flasher image: the second
    /// stage, the reset, then the boot files. `on_event` reports progress and
    /// `cancelled` is checked between steps.
    pub async fn boot(
        &self,
        device: &Identity,
        on_event: &(dyn Fn(BootEvent) + Send + Sync),
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<FileServerOutcome, DriverError> {
        let rom = self
            .config
            .boot_roms
            .iter()
            .find(|rom| rom.model == device.model)
            .ok_or_else(|| {
                DriverError::Unsupported(format!(
                    "{} is not a boot ROM this driver knows",
                    device.model
                ))
            })?;
        let flasher = self.config.flasher.as_deref().ok_or_else(|| {
            DriverError::Incompatible(
                "no flasher image is configured, so this board cannot be booted; put the flasher image's boot files where Atlas can find them"
                    .into(),
            )
        })?;
        boot_device(
            self.bus.as_ref(),
            &device.address,
            rom.chip,
            flasher,
            &self.config.settings,
            on_event,
            cancelled,
        )
        .await
        .map_err(driver_error)
    }
}

fn driver_error(error: RpiBootError) -> DriverError {
    match error {
        RpiBootError::Cancelled => DriverError::Cancelled,
        RpiBootError::Usb(UsbError::Access { message, fix }) => {
            DriverError::Incompatible(format!("{message} {fix}"))
        }
        RpiBootError::Usb(error) => DriverError::Unreachable(error.to_string()),
        other => DriverError::Other(other.to_string()),
    }
}

#[async_trait]
impl Driver for RpiDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if !link.kind.same_kind(&LinkKind::UsbBoot) {
            return Ok(Vec::new());
        }
        let devices = self
            .bus
            .devices()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        Ok(devices
            .iter()
            .filter(|device| self.rom_for(device).is_some())
            .map(|device| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: device.location.clone(),
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let device = self.device_at(&candidate.address).await?;
        let rom = self.rom_for(&device).ok_or_else(|| {
            DriverError::Unreachable(format!(
                "the device at {} is not a boot ROM this driver knows",
                candidate.address
            ))
        })?;
        let serial = device
            .serial
            .clone()
            .unwrap_or_else(|| format!("usb-{}", device.location));
        Ok(Identity {
            key: DeviceKey::board(FAMILY, serial)
                .map_err(|error| DriverError::Incompatible(error.reason))?,
            model: rom.model.clone(),
            state: DeviceState::BootRom,
            lemnos: atlas_types::LemnosState::Absent,
            versions: BTreeMap::new(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: BTreeMap::from([
                ("soc".into(), rom.chip.label().into()),
                (
                    "usb".into(),
                    format!("{:04x}:{:04x}", device.vendor, device.product),
                ),
            ]),
        })
    }

    fn capabilities(&self, _device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds([CapabilityKind::Flash])
    }

    async fn health(&self) -> Vec<HealthCheck> {
        vec![match self.config.flasher {
            Some(_) => HealthCheck::ok(
                "rpi-flasher",
                "Flasher image",
                "the flasher image's boot files are configured",
            ),
            None => HealthCheck::warning(
                "rpi-flasher",
                "Flasher image",
                "no flasher image's boot files are configured, so Raspberry Pi boards cannot be flashed",
                "Put the flasher image's boot files where Atlas can find them.",
            ),
        }]
    }
}
