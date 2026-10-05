//! Everyday actions on a board in recovery, beside flashing:
//!
//! - In USB boot: "Open as USB disk" boots the mass-storage gadget and
//!   writes nothing, so the eMMC can be looked at or backed up.
//! - With the eMMC exposed: "Browse files" mounts it read-only and opens
//!   the file manager; "Eject safely" unmounts and powers it off; "Add my
//!   SSH key" (when a key is set in Atlas) puts it on the boot partition,
//!   where the device package installs it for root at boot.
//! - The bootloader (EEPROM) update, when the device package ships one.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{ActionsCapability, DeviceAction, DriverError, Identity};
use atlas_usbboot::{BootOptions, boot_device};

use crate::RpiConfig;
use crate::boot_files::find_boot_files;
use crate::driver::BOOT_KEYS_PATH;
use crate::eeprom::{EepromUpdate, UPDATE_BOOTLOADER};
use crate::recover::{exposed_disk, storage_port};

const EXPOSE: &str = "open-as-disk";
const BROWSE: &str = "browse-files";
const EJECT: &str = "eject";
const ADD_KEY: &str = "add-ssh-key";

pub(crate) struct RpiActions {
    config: Arc<RpiConfig>,
    eeprom: Option<EepromUpdate>,
}

impl RpiActions {
    pub(crate) fn new(config: Arc<RpiConfig>, has_eeprom: bool) -> Self {
        Self {
            eeprom: has_eeprom.then(|| EepromUpdate::new(config.clone())),
            config,
        }
    }
}

/// `existing` plus the lines of `keys` it doesn't have yet.
fn merge_keys(existing: &str, keys: &str) -> String {
    let mut lines: Vec<&str> = existing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    for key in keys.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if !lines.iter().any(|line| line.trim() == key) {
            lines.push(key);
        }
    }
    let mut merged = lines.join("\n");
    merged.push('\n');
    merged
}

fn action(id: &str, label: &str, destructive: bool) -> DeviceAction {
    DeviceAction {
        id: id.into(),
        label: label.into(),
        destructive,
    }
}

fn unavailable(message: impl Into<String>) -> DriverError {
    DriverError::Unreachable(message.into())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, atlas_blockdev::BlockError> + Send + 'static,
) -> Result<T, DriverError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| DriverError::Other(error.to_string()))?
        .map_err(|error| DriverError::Other(error.to_string()))
}

#[async_trait]
impl ActionsCapability for RpiActions {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
        if storage_port(device).is_some() {
            let mut list = vec![action(BROWSE, "Browse files", false)];
            if self.config.ssh_keys.get().is_some() {
                list.push(action(ADD_KEY, "Add my SSH key", false));
            }
            list.push(action(EJECT, "Eject safely", false));
            return list;
        }
        let mut list = vec![action(EXPOSE, "Open as USB disk", false)];
        if let Some(eeprom) = &self.eeprom {
            list.extend(eeprom.actions(device));
        }
        list
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        match action_id {
            EXPOSE => {
                let files = find_boot_files(&self.config.boot_file_dirs).map_err(|_| {
                    unavailable("USB boot files not found; see Settings > Host health")
                })?;
                let options = BootOptions {
                    location: Some(device.address.clone()),
                    ..BootOptions::default()
                };
                boot_device(&files, &options, &|_| {}, &|| false)
                    .await
                    .map(|_| ())
                    .map_err(|error| unavailable(error.to_string()))
            }
            BROWSE | EJECT | ADD_KEY => {
                let port = storage_port(device)
                    .ok_or_else(|| unavailable("this board's eMMC is not exposed as a disk"))?;
                let disk = exposed_disk(&port).await?;
                if action_id == EJECT {
                    return blocking(move || atlas_blockdev::eject(&disk)).await;
                }
                if action_id == ADD_KEY {
                    let keys =
                        self.config.ssh_keys.get().ok_or_else(|| {
                            unavailable("set your SSH key in Atlas settings first")
                        })?;
                    return blocking(move || {
                        let existing = atlas_blockdev::read_boot_file(&disk, BOOT_KEYS_PATH)?
                            .unwrap_or_default();
                        let merged = merge_keys(&existing, &keys);
                        atlas_blockdev::write_boot_file(&disk, BOOT_KEYS_PATH, &merged)
                    })
                    .await;
                }
                let points = blocking(move || atlas_blockdev::mount_read_only(&disk)).await?;
                for point in &points {
                    let point = point.clone();
                    blocking(move || atlas_blockdev::open_folder(&point)).await?;
                }
                Ok(())
            }
            UPDATE_BOOTLOADER => match &self.eeprom {
                Some(eeprom) => eeprom.run_action(device, action_id).await,
                None => Err(DriverError::Unsupported(
                    "this board's device package has no bootloader files".into(),
                )),
            },
            other => Err(DriverError::Unsupported(format!("no action {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_devices::DeviceCatalog;
    use atlas_driver::{DeviceKey, DeviceMode, LinkId};

    use super::*;

    fn board(stage: Option<&str>) -> Identity {
        let mut attributes = BTreeMap::new();
        if let Some(stage) = stage {
            attributes.insert("stage".to_string(), stage.to_string());
            attributes.insert("usb_port".to_string(), "1-4".to_string());
        }
        Identity {
            key: DeviceKey::new("rpi", "e5226d57"),
            model: "Raze".into(),
            mode: DeviceMode::Recovery,
            versions: BTreeMap::new(),
            name: None,
            link: LinkId("usb-boot".into()),
            address: "001-4".into(),
            attributes,
        }
    }

    fn ids(actions: &RpiActions, device: &Identity) -> Vec<String> {
        actions.actions(device).into_iter().map(|a| a.id).collect()
    }

    #[test]
    fn keys_are_merged_without_duplicates() {
        let merged = merge_keys(
            "ssh-ed25519 AAA a@x\n",
            "ssh-ed25519 AAA a@x\nssh-ed25519 BBB b@y",
        );
        assert_eq!(merged, "ssh-ed25519 AAA a@x\nssh-ed25519 BBB b@y\n");
        assert_eq!(merge_keys("", "ssh-rsa C c"), "ssh-rsa C c\n");
    }

    #[test]
    fn the_key_action_shows_only_with_a_key_set() {
        let config = Arc::new(RpiConfig::default());
        let actions = RpiActions::new(config.clone(), false);
        assert!(!ids(&actions, &board(Some("storage"))).contains(&ADD_KEY.to_string()));
        config.ssh_keys.set(Some("ssh-ed25519 AAA a@x".into()));
        assert!(ids(&actions, &board(Some("storage"))).contains(&ADD_KEY.to_string()));
    }

    #[test]
    fn actions_follow_where_the_board_is() {
        let config = Arc::new(RpiConfig {
            boot_file_dirs: Vec::new(),
            catalog: Arc::new(DeviceCatalog::default()),
            ssh_keys: Default::default(),
        });
        let with_eeprom = RpiActions::new(config.clone(), true);
        assert_eq!(
            ids(&with_eeprom, &board(None)),
            vec![EXPOSE, UPDATE_BOOTLOADER]
        );
        assert_eq!(
            ids(&with_eeprom, &board(Some("storage"))),
            vec![BROWSE, EJECT]
        );
        let plain = RpiActions::new(config, false);
        assert_eq!(ids(&plain, &board(None)), vec![EXPOSE]);
        // Nothing here is destructive except the bootloader update.
        assert!(plain.actions(&board(None)).iter().all(|a| !a.destructive));
    }
}
