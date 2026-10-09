use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage};
use atlas_driver::{
    Candidate, Capabilities, DeviceKey, DeviceMode, Driver, DriverError, DriverManifest, Family,
    HealthCheck, Identity, Link, LinkId, LinkKind, LinkSource,
};
use atlas_usbboot::{
    BROADCOM_VENDOR_ID, BootDevice, Chip, StorageGadget, list_boot_devices, list_storage_gadgets,
};

use crate::actions::RpiActions;
use crate::health;
use crate::recover::RpiRecovery;

pub const RPI_FAMILY: &str = "rpi";
const LINK_ID: &str = "usb-boot";

/// Public SSH keys to leave on a board's boot partition, where the device
/// package installs them for root at boot (`board/authorized_keys`).
/// `None` writes nothing. Shared so the app can change it from settings.
#[derive(Clone, Default)]
pub struct SshKeys(Arc<std::sync::RwLock<Option<String>>>);

impl std::fmt::Debug for SshKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.get().is_some() {
            "SshKeys(set)"
        } else {
            "SshKeys(none)"
        })
    }
}

impl SshKeys {
    /// The OpenSSH public keys in `path` (an `authorized_keys` or `.pub`
    /// file), one per line. Refuses a file with none, such as a private key.
    pub fn read_public(path: &std::path::Path) -> Result<String, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let keys: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| {
                ["ssh-", "ecdsa-", "sk-ssh-", "sk-ecdsa-"]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            })
            .collect();
        if keys.is_empty() {
            return Err(format!(
                "{} is not an OpenSSH public key (choose the .pub file, never the private key)",
                path.display()
            ));
        }
        Ok(keys.join("\n"))
    }

    pub fn set(&self, keys: Option<String>) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = keys;
    }

    pub fn get(&self) -> Option<String> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Where the device package looks for keys on the boot partition.
pub(crate) const BOOT_KEYS_PATH: &str = "board/authorized_keys";

/// Where to look for boot files beyond the defaults, such as the app's
/// bundled resources folder, and which device packages are known.
#[derive(Clone, Debug, Default)]
pub struct RpiConfig {
    pub boot_file_dirs: Vec<PathBuf>,
    /// Device packages, used to name boards in recovery and to find their
    /// EEPROM files and recovery instructions.
    pub catalog: Arc<DeviceCatalog>,
    /// Keys written to the boot partition after a flash, when set.
    pub ssh_keys: SshKeys,
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

    fn name(&self) -> &str {
        "USB"
    }

    fn watch(&self, notify: atlas_driver::ChangeNotifier) -> bool {
        atlas_usbboot::watch_usb(move || notify.notify()).is_ok()
    }

    async fn health(&self) -> Vec<HealthCheck> {
        vec![health::usb_access().await]
    }
}

pub struct RpiDriver {
    manifest: DriverManifest,
    config: Arc<RpiConfig>,
}

/// Candidate addresses of boards in eMMC-as-disk mode start with this.
const STORAGE_PREFIX: &str = "storage:";

impl RpiDriver {
    /// A board that finished USB boot and exposes its eMMC as a USB disk.
    /// It keeps the key it had in USB boot (the boot ROM serial), so it is
    /// the same device in the inventory, now one step further along.
    fn storage_identity(&self, candidate: &Candidate, gadget: &StorageGadget) -> Identity {
        let packages: Vec<&DevicePackage> = self
            .config
            .catalog
            .packages()
            .iter()
            .filter(|package| package.manifest.recovery.storage_target.is_some())
            .collect();
        let display = match packages.as_slice() {
            [package] => package.manifest.display_name().to_string(),
            _ => "Raspberry Pi".to_string(),
        };
        let mut attributes = recovery_attributes(None, &packages);
        attributes.insert("stage".into(), "storage".into());
        attributes.insert("usb_port".into(), gadget.port.clone());
        if let Some(serial) = gadget.board_serial() {
            attributes.insert(atlas_driver::attributes::BOARD_SERIAL.into(), serial);
        }
        Identity {
            key: DeviceKey::new(
                RPI_FAMILY,
                gadget
                    .board_serial()
                    .unwrap_or_else(|| format!("port-{}", gadget.location)),
            ),
            model: format!("{display} (eMMC exposed)"),
            mode: DeviceMode::Recovery,
            versions: BTreeMap::from([("bootloader".to_string(), "storage gadget".to_string())]),
            name: Some(format!("{display} eMMC on USB {}", gadget.location)),
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes,
        }
    }

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
fn recovery_attributes(
    chip: Option<Chip>,
    packages: &[&DevicePackage],
) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    if let Some(chip) = chip {
        attributes.insert("chip".to_string(), chip.label().to_string());
    }
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
        let booting = list_boot_devices().await.map_err(usb_error)?;
        // Boards that already finished USB boot and expose their eMMC.
        let exposed = list_storage_gadgets().await.unwrap_or_default();
        Ok(booting
            .into_iter()
            .map(|device| device.location)
            .chain(
                exposed
                    .into_iter()
                    .map(|gadget| format!("{STORAGE_PREFIX}{}", gadget.location)),
            )
            .map(|address| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address,
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        if let Some(location) = candidate.address.strip_prefix(STORAGE_PREFIX) {
            let gadget = list_storage_gadgets()
                .await
                .map_err(usb_error)?
                .into_iter()
                .find(|gadget| gadget.location == location)
                .ok_or_else(|| DriverError::Unreachable(format!("no Pi eMMC at USB {location}")))?;
            return Ok(self.storage_identity(candidate, &gadget));
        }
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
            address: device.location.clone(),
            attributes: {
                let mut attributes = recovery_attributes(Some(device.chip), &packages);
                let serial = serial_for(&device);
                if !serial.starts_with("port-") {
                    // The last 8 hex digits, as the running OS reports them too.
                    let tail = serial[serial.len().saturating_sub(8)..].to_string();
                    attributes.insert(atlas_driver::attributes::BOARD_SERIAL.into(), tail);
                }
                attributes
            },
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
            actions: Some(Arc::new(RpiActions::new(self.config.clone(), eeprom))
                as Arc<dyn atlas_driver::ActionsCapability>),
            ..Capabilities::default()
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

        let one = recovery_attributes(Some(Chip::Bcm2712), &[&raze]);
        assert_eq!(one.get("model").map(String::as_str), Some("raze"));
        assert_eq!(
            one.get("recovery_steps").map(String::as_str),
            Some("Hold BOOT\nConnect USB-C")
        );

        let many = recovery_attributes(Some(Chip::Bcm2712), &[&raze, &other]);
        assert_eq!(
            many.get("possible_models").map(String::as_str),
            Some("raze, other")
        );
        assert!(!many.contains_key("model"));
    }

    #[test]
    fn an_exposed_emmc_keeps_the_boards_key_and_is_marked_for_direct_write() {
        let catalog = DeviceCatalog::from_packages(vec![package("raze")]);
        let driver = RpiDriver::new(RpiConfig {
            boot_file_dirs: Vec::new(),
            catalog: Arc::new(catalog),
            ssh_keys: Default::default(),
        });
        let gadget = StorageGadget {
            location: "001-4".into(),
            port: "1-4".into(),
            serial: Some("a317bcbee5226d57".into()),
        };
        let candidate = Candidate {
            link: LinkId(LINK_ID.into()),
            family: Family::new(RPI_FAMILY),
            address: format!("{STORAGE_PREFIX}001-4"),
        };
        let identity = driver.storage_identity(&candidate, &gadget);

        // Same key as in USB boot, where the boot ROM reported e5226d57.
        assert_eq!(identity.key, DeviceKey::new(RPI_FAMILY, "e5226d57"));
        assert_eq!(identity.mode, DeviceMode::Recovery);
        assert_eq!(identity.attributes["stage"], "storage");
        assert_eq!(identity.attributes["usb_port"], "1-4");
        assert_eq!(identity.attributes["board_serial"], "e5226d57");
        assert_eq!(identity.attributes["model"], "raze");
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
