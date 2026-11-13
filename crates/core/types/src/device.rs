use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{DeviceKey, LinkId};

/// What a device is doing right now. Each state is a different promise: a
/// device in a boot ROM only takes recovery, a running device takes updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceState {
    /// Booted and running its own software.
    #[default]
    Running,
    /// Waiting in a recovery mode that accepts recovery work only.
    Recovery,
    /// In a boot ROM, before any software has loaded.
    BootRom,
    /// Appears as a removable disk (a card reader or a USB mass-storage gadget).
    RemovableDisk,
    /// A robot's own device on the robot network, read by its driver.
    RobotDevice,
}

impl DeviceState {
    /// Whether the device only takes recovery work in this state.
    pub fn is_recovery(self) -> bool {
        matches!(self, Self::Recovery | Self::BootRom)
    }
}

/// Who a device is, as the device or its driver reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub key: DeviceKey,
    /// Hardware model or board name, as data.
    pub model: String,
    pub state: DeviceState,
    /// Named versions, for example `os`, `firmware`, `bootloader`.
    pub versions: BTreeMap<String, String>,
    /// The name stored on the device, if it has one.
    pub name: Option<String>,
    /// Where the device was reached during this scan.
    pub link: LinkId,
    pub address: String,
    /// Driver-specific facts shown in the device panel.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

impl Identity {
    /// The version shown in the inventory: `os`, then `firmware`, then `bootloader`.
    pub fn primary_version(&self) -> Option<&str> {
        ["os", "firmware", "bootloader"]
            .iter()
            .find_map(|name| self.versions.get(*name))
            .map(String::as_str)
    }
}
