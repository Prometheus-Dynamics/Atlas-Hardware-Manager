use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage};
use atlas_driver::{
    Candidate, Capabilities, DeviceKey, DeviceMode, Driver, DriverError, DriverManifest, Family,
    HealthCheck, Identity, Link, LinkId, LinkKind, LinkSource,
};
use atlas_usbboot::{BROADCOM_VENDOR_ID, BootDevice, Chip, list_boot_devices};

use crate::eeprom::EepromUpdate;
use crate::health;
use crate::recover::RpiRecovery;

pub const RPI_FAMILY: &str = "rpi";
const LINK_ID: &str = "usb-boot";

/// Where to look for boot files beyond the defaults, such as the app's
/// bundled resources folder, and which device packages are known.
#[derive(Clone, Debug, Default)]
pub struct RpiConfig {
    pub boot_file_dirs: Vec<PathBuf>,
    /// Device packages, used to name boards in recovery and to find their
    /// EEPROM files and recovery instructions.
    pub catalog: Arc<DeviceCatalog>,
}

/// The single link for every Pi in USB boot mode on this computer.
pub struct UsbBootLinks;

#[async_trait]
impl LinkSource for UsbBootLinks {
    async fn links(&self) -> Vec<Link> {
        vec![Link {
            id: LinkId(LINK_ID.into()),
            kind: LinkKind::UsbBoot,
            label: "USB boot".into(),
        }]
    }

    async fn health(&self) -> Vec<HealthCheck> {
        vec![health::usb_access().await]
    }
}

pub struct RpiDriver {
    manifest: DriverManifest,
    config: Arc<RpiConfig>,
}

impl RpiDriver {
    pub fn new(config: RpiConfig) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(RPI_FAMILY),
                name: "Raspberry Pi (USB boot)".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbBoot],
                priority: 0,
            },
            config: Arc::new(config),
        }
    }
}

fn short_chip(chip: Chip) -> &'static str {
    match chip {
        Chip::Bcm2835 => "Pi 1 / Zero",
        Chip::Bcm2837 => "Pi 3 / CM3",
        Chip::Bcm2711 => "CM4 / Pi 4",
        Chip::Bcm2712 => "CM5 / Pi 5",
    }
}

/// A boot ROM serial when it looks like a real board serial, else the port.
/// Boot ROMs do not all report one, and a port key is stable while the
/// board stays plugged in, which is all a recovery needs.
fn serial_for(device: &BootDevice) -> String {
    device
        .serial
        .as_deref()
        .map(str::trim)
        .filter(|serial| serial.len() >= 6 && serial.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|serial| serial.to_ascii_lowercase())
        .unwrap_or_else(|| format!("port-{}", device.location))
}

/// What the device packages say about a board in recovery: its model when
/// exactly one package uses this chip, the candidates otherwise, and the
/// package's recovery steps.
fn recovery_attributes(chip: Chip, packages: &[&DevicePackage]) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::from([("chip".to_string(), chip.label().to_string())]);
    match packages {
        [package] => {
            let manifest = &package.manifest;
            attributes.insert("model".into(), manifest.model.clone());
            if let Some(target) = &manifest.recovery.storage_target {
                attributes.insert("storage".into(), target.clone());
            }
            if !manifest.recovery.instructions.is_empty() {
                attributes.insert(
                    "recovery_steps".into(),
                    manifest.recovery.instructions.join("\n"),
                );
            }
        }
        [] => {}
        several => {
            let models: Vec<&str> = several
                .iter()
                .map(|package| package.manifest.model.as_str())
                .collect();
            attributes.insert("possible_models".into(), models.join(", "));
        }
    }
    attributes
}

fn usb_error(error: atlas_usbboot::UsbBootError) -> DriverError {
    match error.fix() {
        Some(fix) => DriverError::Unreachable(format!("{error} {fix}")),
        None => DriverError::Unreachable(error.to_string()),
    }
}

#[async_trait]
impl Driver for RpiDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if link.kind != LinkKind::UsbBoot {
            return Ok(Vec::new());
        }
        Ok(list_boot_devices()
            .await
            .map_err(usb_error)?
            .into_iter()
            .map(|device| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: device.location,
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let device = list_boot_devices()
            .await
            .map_err(usb_error)?
            .into_iter()
            .find(|device| device.location == candidate.address)
            .ok_or_else(|| {
                DriverError::Unreachable(format!("no Pi at USB {}", candidate.address))
            })?;
        let packages = self
            .config
            .catalog
            .for_usb_boot(BROADCOM_VENDOR_ID, device.chip.product_id());
        Ok(Identity {
            key: DeviceKey::new(RPI_FAMILY, serial_for(&device)),
            model: match packages.as_slice() {
                [package] => format!(
                    "{} ({})",
                    package.manifest.display_name(),
                    device.chip.label()
                ),
                _ => device.chip.label().into(),
            },
            mode: DeviceMode::Recovery,
            versions: BTreeMap::from([("bootloader".to_string(), "usb boot".to_string())]),
            name: Some(format!(
                "{} on USB {}",
                match packages.as_slice() {
                    [package] => package.manifest.display_name(),
                    _ => short_chip(device.chip),
                },
                device.location
            )),
            link: candidate.link.clone(),
            address: device.location,
            attributes: recovery_attributes(device.chip, &packages),
        })
    }

    fn capabilities(&self, device: &Identity) -> Capabilities {
        let eeprom = device
            .attributes
            .get("model")
            .and_then(|model| self.config.catalog.by_model(model))
            .is_some_and(|package| package.eeprom_dir().is_some());
        Capabilities {
            update: Some(Arc::new(RpiRecovery::new(self.config.clone()))),
            actions: eeprom.then(|| {
                Arc::new(EepromUpdate::new(self.config.clone()))
                    as Arc<dyn atlas_driver::ActionsCapability>
            }),
        }
    }

    async fn health(&self) -> Vec<HealthCheck> {
        health::driver_checks(&self.config).await
    }

    async fn fix(&self, action: &str) -> Result<String, DriverError> {
        health::fix(action).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(serial: Option<&str>) -> BootDevice {
        BootDevice {
            location: "1-2.3".into(),
            chip: Chip::Bcm2711,
            serial: serial.map(str::to_string),
            needs_driver: false,
        }
    }

    fn package(model: &str) -> DevicePackage {
        DevicePackage {
            manifest: serde_json::from_value(serde_json::json!({
                "contract": 1,
                "model": model,
                "recovery": {
                    "usb_boot": { "vid": "0a5c", "pid": "2712" },
                    "storage_target": "emmc",
                    "instructions": ["Hold BOOT", "Connect USB-C"]
                }
            }))
            .unwrap(),
            compat: Default::default(),
            dir: PathBuf::from("/nonexistent"),
        }
    }

    #[test]
    fn one_matching_package_names_the_board_and_several_are_listed() {
        let raze = package("raze");
        let other = package("other");

        let one = recovery_attributes(Chip::Bcm2712, &[&raze]);
        assert_eq!(one.get("model").map(String::as_str), Some("raze"));
        assert_eq!(
            one.get("recovery_steps").map(String::as_str),
            Some("Hold BOOT\nConnect USB-C")
        );

        let many = recovery_attributes(Chip::Bcm2712, &[&raze, &other]);
        assert_eq!(
            many.get("possible_models").map(String::as_str),
            Some("raze, other")
        );
        assert!(!many.contains_key("model"));
    }

    #[test]
    fn real_serials_are_used_and_placeholders_fall_back_to_the_port() {
        assert_eq!(
            serial_for(&device(Some("10000000ABCDEF01"))),
            "10000000abcdef01"
        );
        assert_eq!(serial_for(&device(Some("0"))), "port-1-2.3");
        assert_eq!(serial_for(&device(Some("Broadcom"))), "port-1-2.3");
        assert_eq!(serial_for(&device(None)), "port-1-2.3");
    }
}
