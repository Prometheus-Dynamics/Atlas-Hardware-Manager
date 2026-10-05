//! Everyday actions on a board in recovery, beside flashing:
//!
//! - In USB boot: "Open as USB disk" boots the mass-storage gadget and
//!   writes nothing, so the eMMC can be looked at or backed up.
//! - With the eMMC exposed: "Browse files" mounts it read-only and opens
//!   the file manager; "Eject safely" unmounts and powers it off.
//! - The bootloader (EEPROM) update, when the device package ships one.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{ActionsCapability, DeviceAction, DriverError, Identity};
use atlas_usbboot::{BootOptions, boot_device};

use crate::RpiConfig;
use crate::boot_files::find_boot_files;
use crate::eeprom::{EepromUpdate, UPDATE_BOOTLOADER};
use crate::recover::{exposed_disk, storage_port};

const EXPOSE: &str = "open-as-disk";
const BROWSE: &str = "browse-files";
const EJECT: &str = "eject";

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
            return vec![
                action(BROWSE, "Browse files", false),
                action(EJECT, "Eject safely", false),
            ];
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
            BROWSE | EJECT => {
                let port = storage_port(device)
                    .ok_or_else(|| unavailable("this board's eMMC is not exposed as a disk"))?;
                let disk = exposed_disk(&port).await?;
                if action_id == EJECT {
                    return blocking(move || atlas_blockdev::eject(&disk)).await;
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
    fn actions_follow_where_the_board_is() {
        let config = Arc::new(RpiConfig {
            boot_file_dirs: Vec::new(),
            catalog: Arc::new(DeviceCatalog::default()),
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
