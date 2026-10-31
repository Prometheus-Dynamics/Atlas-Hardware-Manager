//! Finding boards in a SoC download mode and naming them from the catalog.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_devices::{DeviceCatalog, DevicePackage};
use atlas_driver::{
    Candidate, Capabilities, DeviceKey, DeviceMode, Driver, DriverError, DriverManifest, Family,
    Identity, Link, LinkKind,
};
use atlas_socflash::{k230, rockchip};

pub const ROCKCHIP_FAMILY: &str = "rockchip";
pub const K230_FAMILY: &str = "k230";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Soc {
    Rockchip,
    K230,
}

/// A board in a download mode on this computer's USB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub location: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// `maskrom`, `loader` or `usb boot`.
    pub stage: &'static str,
    pub serial: Option<String>,
}

pub(crate) async fn list(
    soc: Soc,
) -> Result<Vec<(Found, atlas_socflash::transport::DeviceInfo)>, DriverError> {
    let unreachable = |error: atlas_socflash::UsbError| DriverError::Unreachable(error.to_string());
    Ok(match soc {
        Soc::Rockchip => rockchip::list()
            .await
            .map_err(unreachable)?
            .into_iter()
            .map(|(device, info)| {
                let found = Found {
                    location: device.location,
                    vendor_id: rockchip::VENDOR_ID,
                    product_id: device.product_id,
                    stage: match device.mode {
                        rockchip::Mode::Maskrom => "maskrom",
                        rockchip::Mode::Loader => "loader",
                    },
                    serial: device.serial,
                };
                (found, info)
            })
            .collect(),
        Soc::K230 => k230::list()
            .await
            .map_err(unreachable)?
            .into_iter()
            .map(|(device, info)| {
                let found = Found {
                    location: device.location,
                    vendor_id: k230::VENDOR_ID,
                    product_id: k230::PRODUCT_ID,
                    stage: "usb boot",
                    serial: device.serial,
                };
                (found, info)
            })
            .collect(),
    })
}

/// The chip a Rockchip download-mode PID stands for, where it's known.
fn rockchip_chip(product_id: u16) -> Option<&'static str> {
    match product_id {
        0x350a => Some("RK3566/RK3568"),
        0x350b => Some("RK3588"),
        0x350e => Some("RK3576"),
        0x110c => Some("RV1106/RV1103"),
        0x330c => Some("RK3399"),
        0x320c => Some("RK3328"),
        _ => None,
    }
}

pub struct SocDriver {
    soc: Soc,
    manifest: DriverManifest,
    catalog: Arc<DeviceCatalog>,
}

impl SocDriver {
    pub fn new(soc: Soc, catalog: Arc<DeviceCatalog>) -> Self {
        let (family, name) = match soc {
            Soc::Rockchip => (ROCKCHIP_FAMILY, "Rockchip (maskrom)"),
            Soc::K230 => (K230_FAMILY, "Kendryte K230 (USB boot)"),
        };
        Self {
            soc,
            manifest: DriverManifest {
                family: Family::new(family),
                name: name.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbBoot],
                priority: 0,
            },
            catalog,
        }
    }

    fn chip(&self, found: &Found) -> String {
        match self.soc {
            Soc::Rockchip => rockchip_chip(found.product_id).map_or_else(
                || format!("Rockchip {:04x}", found.product_id),
                str::to_string,
            ),
            Soc::K230 => "K230".into(),
        }
    }

    pub(crate) fn identity(&self, candidate: &Candidate, found: &Found) -> Identity {
        let mut packages = self.catalog.for_usb_boot(found.vendor_id, found.product_id);
        // A PID no record names may be a board whose PID is still a guess
        // (RK3576's maskrom); its chip is checked before anything is written.
        let provisional = packages.is_empty() && self.soc == Soc::Rockchip;
        if provisional {
            packages = self.catalog.unconfirmed_usb_boot(found.vendor_id);
        }
        let chip = self.chip(found);
        let mut attributes = BTreeMap::from([
            ("chip".to_string(), chip.clone()),
            ("usb_port".to_string(), found.location.clone()),
            ("stage".to_string(), found.stage.to_string()),
        ]);
        let display = match packages.as_slice() {
            [package] => {
                describe(package, &mut attributes);
                if provisional {
                    attributes.insert(
                        "pid_unconfirmed".into(),
                        format!(
                            "its USB id {:04x}:{:04x} isn't one the record confirms; Atlas checks the chip the board reports before writing",
                            found.vendor_id, found.product_id
                        ),
                    );
                }
                package.manifest.display_name().to_string()
            }
            [] => chip.clone(),
            several => {
                let models: Vec<&str> = several.iter().map(|p| p.manifest.model.as_str()).collect();
                attributes.insert("possible_models".into(), models.join(", "));
                chip.clone()
            }
        };
        let serial = found
            .serial
            .as_deref()
            .filter(|serial| serial.len() >= 6 && serial.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| format!("port-{}", found.location));
        Identity {
            key: DeviceKey::new(self.manifest.family.0.clone(), serial),
            model: if display == chip {
                chip.clone()
            } else {
                format!("{display} ({chip})")
            },
            mode: DeviceMode::Recovery,
            versions: BTreeMap::from([("bootloader".to_string(), found.stage.to_string())]),
            name: Some(format!("{display} on USB {}", found.location)),
            link: candidate.link.clone(),
            address: found.location.clone(),
            attributes,
        }
    }
}

/// What the board record says, as the device's attributes.
fn describe(package: &DevicePackage, attributes: &mut BTreeMap<String, String>) {
    let recovery = &package.manifest.recovery;
    attributes.insert("model".into(), package.manifest.model.clone());
    if let Some(storage) = &recovery.storage_target {
        attributes.insert("storage".into(), storage.clone());
    }
    if !recovery.instructions.is_empty() {
        attributes.insert("recovery_steps".into(), recovery.instructions.join("\n"));
    }
    if recovery.untested() {
        attributes.insert(
            "flashing".into(),
            "written from the vendor's protocol, not yet tested on this board".into(),
        );
    }
}

/// The Rockchip and K230 drivers over `catalog`'s board records.
pub fn drivers(catalog: Arc<DeviceCatalog>) -> Vec<Arc<dyn Driver>> {
    vec![
        Arc::new(SocDriver::new(Soc::Rockchip, catalog.clone())),
        Arc::new(SocDriver::new(Soc::K230, catalog)),
    ]
}

#[async_trait]
impl Driver for SocDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if link.kind != LinkKind::UsbBoot {
            return Ok(Vec::new());
        }
        Ok(list(self.soc)
            .await?
            .into_iter()
            .map(|(found, _)| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: found.location,
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let found = list(self.soc)
            .await?
            .into_iter()
            .map(|(found, _)| found)
            .find(|found| found.location == candidate.address)
            .ok_or_else(|| {
                DriverError::Unreachable(format!(
                    "nothing in download mode at USB {}",
                    candidate.address
                ))
            })?;
        Ok(self.identity(candidate, &found))
    }

    fn capabilities(&self, _device: &Identity) -> Capabilities {
        Capabilities {
            update: Some(Arc::new(crate::flash::SocUpdate {
                soc: self.soc,
                catalog: self.catalog.clone(),
            })),
            ..Capabilities::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_driver::LinkId;

    fn catalog() -> Arc<DeviceCatalog> {
        let devices = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../devices");
        Arc::new(DeviceCatalog::load(&[devices]))
    }

    fn candidate(address: &str) -> Candidate {
        Candidate {
            link: LinkId("usb-boot".into()),
            family: Family::new(ROCKCHIP_FAMILY),
            address: address.into(),
        }
    }

    #[test]
    fn a_rock_5c_in_maskrom_is_named_from_its_record() {
        let driver = SocDriver::new(Soc::Rockchip, catalog());
        let found = Found {
            location: "3-2".into(),
            vendor_id: 0x2207,
            product_id: 0x350b,
            stage: "maskrom",
            serial: None,
        };
        let identity = driver.identity(&candidate("3-2"), &found);
        assert_eq!(identity.model, "ROCK 5C (RK3588)");
        assert_eq!(identity.key, DeviceKey::new("rockchip", "port-3-2"));
        assert_eq!(identity.mode, DeviceMode::Recovery);
        assert_eq!(identity.attributes["model"], "radxa-rock-5c");
        assert_eq!(identity.attributes["storage"], "emmc");
        assert!(identity.attributes["flashing"].contains("not yet tested"));
        assert!(identity.attributes["recovery_steps"].contains("Maskrom pin hole"));
        assert_eq!(identity.versions["bootloader"], "maskrom");
    }

    #[test]
    fn a_rockchip_no_record_names_is_offered_as_the_unconfirmed_one() {
        let driver = SocDriver::new(Soc::Rockchip, catalog());
        let found = Found {
            location: "2-1".into(),
            vendor_id: 0x2207,
            product_id: 0x350f,
            stage: "maskrom",
            serial: None,
        };
        let identity = driver.identity(&candidate("2-1"), &found);
        assert_eq!(identity.attributes["model"], "radxa-rock-4d");
        assert!(identity.attributes["pid_unconfirmed"].contains("2207:350f"));
    }

    #[test]
    fn an_unknown_rockchip_is_its_chip_and_a_k230_its_record() {
        let driver = SocDriver::new(Soc::Rockchip, catalog());
        let found = Found {
            location: "1-1".into(),
            vendor_id: 0x2207,
            product_id: 0x350a,
            stage: "loader",
            serial: Some("ABCDEF123".into()),
        };
        let identity = driver.identity(&candidate("1-1"), &found);
        assert_eq!(identity.key, DeviceKey::new("rockchip", "abcdef123"));
        // No record names 350a, so the unconfirmed record is offered, with
        // the chip the PID says.
        assert_eq!(identity.model, "ROCK 4D (RK3566/RK3568)");
        assert!(identity.attributes.contains_key("pid_unconfirmed"));

        let driver = SocDriver::new(Soc::K230, catalog());
        let found = Found {
            location: "1-4".into(),
            vendor_id: 0x29f1,
            product_id: 0x0230,
            stage: "usb boot",
            serial: None,
        };
        let identity = driver.identity(&candidate("1-4"), &found);
        assert_eq!(identity.model, "CanMV K230D Zero (K230)");
        assert_eq!(identity.attributes["storage"], "sd");
    }
}
