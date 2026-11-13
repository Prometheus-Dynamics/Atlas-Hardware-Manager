use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::DeviceState;

/// Everything a device can offer. The UI shows a tab or action only for the
/// kinds a device reports. Declaration order is the display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKind {
    /// Who the device is. Every device has it, so it is never granted by hand.
    Info,
    /// Moves the device to a release, from a running state.
    Update,
    /// Moves the device to a release, from a recovery state. Shown instead of
    /// `Update` while the device is in recovery.
    Recover,
    /// The device's settings can be read and changed.
    Config,
    /// The device keeps a log.
    Logs,
    /// The device reports measurements.
    Telemetry,
    /// The device has named one-shot actions, such as locate or reboot.
    Actions,
    /// The device relays to other devices.
    Gateway,
    /// The device serves a web page of its own.
    OpenUi,
    /// The device can check its own hardware.
    SelfTest,
    /// The device reports its boot, health and update state.
    Status,
    /// The device's parts take commands, such as setting or releasing controls.
    HardwareControl,
    /// The device keeps several OS images side by side.
    Slots,
    /// The device takes packages through an install method of its own.
    Install,
}

impl CapabilityKind {
    /// Kinds that only make sense while the device runs its own software.
    fn running_only(self) -> bool {
        matches!(
            self,
            Self::SelfTest | Self::Status | Self::HardwareControl | Self::Slots | Self::Install
        )
    }
}

/// The capability kinds one driver grants one device. Absent means unsupported.
///
/// This is the only way a device's features are described to the rest of
/// Atlas. A driver grants kinds; the state of the device decides what is shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    kinds: BTreeSet<CapabilityKind>,
}

impl CapabilitySet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a set from kinds. `Info` is implied and need not be listed.
    pub fn from_kinds(kinds: impl IntoIterator<Item = CapabilityKind>) -> Self {
        Self {
            kinds: kinds.into_iter().collect(),
        }
    }

    pub fn grant(&mut self, kind: CapabilityKind) -> &mut Self {
        self.kinds.insert(kind);
        self
    }

    /// Whether the driver granted this kind, whatever the state.
    pub fn grants(&self, kind: CapabilityKind) -> bool {
        self.kinds.contains(&kind)
    }

    /// The kinds to show for a device in `state`, `Info` first and then in
    /// declaration order, without repeats.
    ///
    /// - `Update` is shown as `Recover` in a recovery state, and `Recover` is
    ///   shown only there.
    /// - Kinds that only make sense on a running device are hidden otherwise.
    pub fn shown(&self, state: DeviceState) -> Vec<CapabilityKind> {
        let mut shown = BTreeSet::from([CapabilityKind::Info]);
        for &kind in &self.kinds {
            let mapped = match kind {
                CapabilityKind::Update if state.is_recovery() => Some(CapabilityKind::Recover),
                CapabilityKind::Update => Some(CapabilityKind::Update),
                CapabilityKind::Recover if state.is_recovery() => Some(CapabilityKind::Recover),
                CapabilityKind::Recover => None,
                other if other.running_only() && state != DeviceState::Running => None,
                other => Some(other),
            };
            shown.extend(mapped);
        }
        shown.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CapabilityKind::*;

    fn sample() -> CapabilitySet {
        CapabilitySet::from_kinds([Update, Recover, Logs, Status, Slots, Actions])
    }

    #[test]
    fn info_is_always_first() {
        let set = CapabilitySet::new();
        assert_eq!(set.shown(DeviceState::Running), vec![Info]);
        assert_eq!(sample().shown(DeviceState::Running)[0], Info);
    }

    #[test]
    fn running_device_shows_update_and_running_kinds() {
        assert_eq!(
            sample().shown(DeviceState::Running),
            vec![Info, Update, Logs, Actions, Status, Slots]
        );
    }

    #[test]
    fn recovery_shows_recover_instead_of_update() {
        assert_eq!(
            sample().shown(DeviceState::Recovery),
            vec![Info, Recover, Logs, Actions]
        );
        assert_eq!(
            sample().shown(DeviceState::BootRom),
            vec![Info, Recover, Logs, Actions]
        );
    }

    #[test]
    fn removable_disk_keeps_update_and_hides_running_only_kinds() {
        assert_eq!(
            sample().shown(DeviceState::RemovableDisk),
            vec![Info, Update, Logs, Actions]
        );
    }

    #[test]
    fn grants_reports_what_was_granted() {
        let set = sample();
        assert!(set.grants(Logs));
        assert!(!set.grants(Telemetry));
    }
}
