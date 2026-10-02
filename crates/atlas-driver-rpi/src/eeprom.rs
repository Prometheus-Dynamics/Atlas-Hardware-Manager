//! Bootloader (EEPROM) update from a device package's `eeprom/` folder.
//!
//! An EEPROM update is another USB boot: the second stage is the flashing
//! tool (`recovery.bin`), which asks for `pieeprom.bin` and `pieeprom.sig`
//! by name and writes them to the board's SPI flash. Packages name their
//! files by version and mark them with a role, so Atlas stages a folder with
//! the names the tool expects before booting.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use atlas_devices::DevicePackage;
use atlas_driver::{ActionsCapability, DeviceAction, DriverError, Identity};
use atlas_usbboot::{BootFiles, BootOptions, boot_device};
use sha2::{Digest, Sha256};

use crate::RpiConfig;

pub(crate) const UPDATE_BOOTLOADER: &str = "update-bootloader";

/// Served to the recovery tool: report OTP metadata (serial, MAC, board
/// revision) so Atlas learns the board while it updates it.
const RECOVERY_CONFIG: &str = "recovery_metadata=1\n";

pub(crate) struct EepromUpdate {
    config: Arc<RpiConfig>,
}

impl EepromUpdate {
    pub(crate) fn new(config: Arc<RpiConfig>) -> Self {
        Self { config }
    }
}

fn failed(message: impl Into<String>) -> DriverError {
    DriverError::StepFailed {
        step: "apply".into(),
        message: message.into(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Copies one role's file into the staging folder under each `names`,
/// checking its SHA-256 when the manifest gives one.
fn stage_role(
    package: &DevicePackage,
    role: &str,
    names: &[&str],
    staging: &Path,
) -> Result<bool, DriverError> {
    let Some(file) = package.manifest.eeprom_file(role) else {
        return Ok(false);
    };
    let source = package.dir.join(&file.path);
    let data = std::fs::read(&source).map_err(|error| {
        failed(format!(
            "{} is missing ({error}); fetch it as listed in the package's fetch.lock",
            source.display()
        ))
    })?;
    if let Some(expected) = file.sha256.as_deref().filter(|sha| !sha.is_empty()) {
        let actual = hex(&Sha256::digest(&data));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(failed(format!(
                "{} failed its SHA-256 check (expected {expected}, got {actual}); the package \
                 file is corrupt",
                source.display()
            )));
        }
    }
    for name in names {
        std::fs::write(staging.join(name), &data)
            .map_err(|error| failed(format!("could not stage {name}: {error}")))?;
    }
    Ok(true)
}

/// Builds the folder the recovery tool boots from. Packages without file
/// roles are used as they are (a `raspberrypi/usbboot` style folder).
pub(crate) fn stage(package: &DevicePackage, staging: &Path) -> Result<PathBuf, DriverError> {
    let dir = package
        .eeprom_dir()
        .ok_or_else(|| failed("the device package has no eeprom/ folder"))?;
    let has_roles = package
        .manifest
        .eeprom
        .as_ref()
        .is_some_and(|eeprom| eeprom.files.iter().any(|file| file.role.is_some()));
    if !has_roles {
        return Ok(dir);
    }
    std::fs::create_dir_all(staging)
        .map_err(|error| failed(format!("could not create a staging folder: {error}")))?;
    if !stage_role(package, "recovery", &["recovery.bin"], staging)? {
        return Err(failed(
            "the device package does not list recovery.bin (role \"recovery\")",
        ));
    }
    if !stage_role(
        package,
        "pieeprom",
        &["pieeprom.bin", "pieeprom.upd"],
        staging,
    )? {
        return Err(failed(
            "the device package does not list a bootloader image (role \"pieeprom\")",
        ));
    }
    stage_role(package, "pieeprom-sig", &["pieeprom.sig"], staging)?;
    std::fs::write(staging.join("config.txt"), RECOVERY_CONFIG)
        .map_err(|error| failed(format!("could not stage config.txt: {error}")))?;
    Ok(staging.to_path_buf())
}

#[async_trait]
impl ActionsCapability for EepromUpdate {
    fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
        vec![DeviceAction {
            id: UPDATE_BOOTLOADER.into(),
            label: "Update bootloader (EEPROM)".into(),
            destructive: true,
        }]
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        if action_id != UPDATE_BOOTLOADER {
            return Err(DriverError::Unsupported(format!("no action {action_id}")));
        }
        let package = device
            .attributes
            .get("model")
            .and_then(|model| self.config.catalog.by_model(model))
            .ok_or_else(|| {
                DriverError::Unsupported("no device package is known for this board".into())
            })?;
        let staging = std::env::temp_dir().join(format!(
            "atlas-eeprom-{}-{}",
            std::process::id(),
            device.address.replace(['/', '\\', ':'], "_")
        ));
        let dir = stage(package, &staging)?;
        let result = async {
            let files = BootFiles::open(&dir).map_err(|error| failed(error.to_string()))?;
            let options = BootOptions {
                location: Some(device.address.clone()),
                ..BootOptions::default()
            };
            boot_device(&files, &options, &|_| {}, &|| false)
                .await
                .map(|_| ())
                .map_err(|error| {
                    let fix = error.fix().map(|fix| format!(" {fix}")).unwrap_or_default();
                    failed(format!("{error}.{fix}"))
                })
        }
        .await;
        if dir == staging {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atlas-eeprom-test-{label}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(dir.join("eeprom")).unwrap();
        dir
    }

    fn package(dir: &Path, image_sha: &str) -> DevicePackage {
        DevicePackage {
            manifest: serde_json::from_value(serde_json::json!({
                "contract": 1,
                "model": "raze",
                "eeprom": { "files": [
                    { "path": "eeprom/pieeprom-2025.upd", "role": "pieeprom", "sha256": image_sha },
                    { "path": "eeprom/pieeprom-2025.sig", "role": "pieeprom-sig" },
                    { "path": "eeprom/recovery.bin", "role": "recovery" }
                ]}
            }))
            .unwrap(),
            compat: Default::default(),
            dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn roles_are_staged_under_the_names_the_tool_requests() {
        let dir = package_dir("stage");
        std::fs::write(dir.join("eeprom/pieeprom-2025.upd"), b"image").unwrap();
        std::fs::write(dir.join("eeprom/pieeprom-2025.sig"), b"digest").unwrap();
        std::fs::write(dir.join("eeprom/recovery.bin"), b"tool").unwrap();
        let image_sha = hex(&Sha256::digest(b"image"));
        let staging = dir.join("staging");

        let staged = stage(&package(&dir, &image_sha), &staging).unwrap();

        assert_eq!(
            std::fs::read(staged.join("pieeprom.bin")).unwrap(),
            b"image"
        );
        assert_eq!(
            std::fs::read(staged.join("pieeprom.sig")).unwrap(),
            b"digest"
        );
        assert_eq!(std::fs::read(staged.join("recovery.bin")).unwrap(), b"tool");
        assert!(
            std::fs::read_to_string(staged.join("config.txt"))
                .unwrap()
                .contains("recovery_metadata=1")
        );
        assert!(BootFiles::open(&staged).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_tool_or_corrupt_image_stops_the_update() {
        let dir = package_dir("missing");
        std::fs::write(dir.join("eeprom/pieeprom-2025.upd"), b"image").unwrap();

        let missing_tool = stage(&package(&dir, ""), &dir.join("staging"));
        assert!(missing_tool.is_err());

        std::fs::write(dir.join("eeprom/recovery.bin"), b"tool").unwrap();
        let corrupt = stage(&package(&dir, &"00".repeat(32)), &dir.join("staging2"));
        assert!(
            matches!(corrupt, Err(DriverError::StepFailed { message, .. }) if message.contains("SHA-256"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
