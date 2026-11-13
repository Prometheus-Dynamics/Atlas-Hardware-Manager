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

/// What a board's Lemnos (its own program) is doing, as far as Atlas can see it.
///
/// `Absent` is normal: a node that never ran Lemnos is a plain Orion node, with no slots or
/// hardware pages. The other three are about a node that is expected to run Lemnos: it has
/// published Lemnos facts or offers before, or its board description says it runs Lemnos.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum LemnosState {
    /// The node never ran Lemnos. Not a fault.
    #[default]
    Absent,
    /// Lemnos answers.
    Answering,
    /// Expected, and not answering: faulted and restarting. `reason` is the last error.
    NotAnswering { reason: String },
    /// Lemnos refused because its client and daemon were built from different revisions. The
    /// app or the board needs updating. Not a fault. `reason` is Lemnos's own words.
    RevisionMismatch { reason: String },
}

impl LemnosState {
    /// The reason behind a state that has one, shown beside it.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::NotAnswering { reason } | Self::RevisionMismatch { reason } => Some(reason),
            Self::Absent | Self::Answering => None,
        }
    }
}

/// Who a device is, as the device or its driver reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub key: DeviceKey,
    /// Hardware model or board name, as data.
    pub model: String,
    pub state: DeviceState,
    /// The board's Lemnos state. Absent for a node that never ran Lemnos.
    #[serde(default)]
    pub lemnos: LemnosState,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_lemnos_state_says_its_reason_only_when_it_has_one() {
        assert_eq!(LemnosState::Absent.reason(), None);
        assert_eq!(LemnosState::Answering.reason(), None);
        let faulted = LemnosState::NotAnswering {
            reason: "-4: not answering".into(),
        };
        assert_eq!(faulted.reason(), Some("-4: not answering"));
        let mismatch = LemnosState::RevisionMismatch {
            reason: "different Lemnos revisions".into(),
        };
        assert_eq!(mismatch.reason(), Some("different Lemnos revisions"));
    }

    #[test]
    fn a_lemnos_state_round_trips_through_its_stored_form() {
        for state in [
            LemnosState::Absent,
            LemnosState::Answering,
            LemnosState::NotAnswering {
                reason: "gone".into(),
            },
            LemnosState::RevisionMismatch {
                reason: "old".into(),
            },
        ] {
            let stored = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<LemnosState>(&stored).unwrap(), state);
        }
    }

    #[test]
    fn an_identity_written_before_it_had_a_lemnos_state_loads_as_absent() {
        let before = r#"{
            "key": {"family": "sample-soc", "serial": "1000abcd"},
            "model": "sample-board",
            "state": "running",
            "versions": {"os": "1.0"},
            "name": null,
            "link": "usbnet:a",
            "address": "10.0.0.2"
        }"#;
        let identity: Identity = serde_json::from_str(before).unwrap();
        assert_eq!(identity.key, DeviceKey::new("sample-soc", "1000abcd"));
        assert_eq!(identity.lemnos, LemnosState::Absent);
        assert!(identity.attributes.is_empty());
    }
}
