use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::DeviceState;

/// Everything a device can offer. The UI shows a tab or action only for the
/// kinds a device reports and that apply in its current state (see
/// [`CapabilityKind::applies_in`]). Declaration order is the display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKind {
    /// Who the device is. Every device has it, so it is never granted by hand.
    Info,
    /// Moves the device to a release, from a running state.
    Update,
    /// Moves the device to a release, from a recovery state. A driver grants
    /// `Update`; in recovery it is shown as `Recover` instead.
    Recover,
    /// The device's settings can be read and changed.
    Config,
    /// The device keeps a log.
    Logs,
    /// The device reports measurements.
    Telemetry,
    /// The device has named one-shot actions, such as locate or reboot.
    Actions,
    /// An Orion node that forwards to boards plugged into it. This is not an
    /// HTTP gateway: the device relays Orion's own wire to the boards behind it.
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
    /// Reads and writes files on the device.
    Files,
    /// A terminal for a person: SSH on a running board, the recovery shell in recovery.
    Shell,
    /// The device can be written from its boot ROM or as a removable disk.
    Flash,
}

impl CapabilityKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 17] = [
        Self::Info,
        Self::Update,
        Self::Recover,
        Self::Config,
        Self::Logs,
        Self::Telemetry,
        Self::Actions,
        Self::Gateway,
        Self::OpenUi,
        Self::SelfTest,
        Self::Status,
        Self::HardwareControl,
        Self::Slots,
        Self::Install,
        Self::Files,
        Self::Shell,
        Self::Flash,
    ];

    /// The one table of which device states each kind applies in. Adding a
    /// kind means adding an arm here, and the table test then needs its row.
    pub const fn applies_in(self, state: DeviceState) -> bool {
        use DeviceState::{BootRom, Recovery, RemovableDisk, RobotDevice, Running};
        match self {
            Self::Info => true,
            Self::Update => matches!(state, Running),
            Self::Recover => matches!(state, Recovery),
            Self::Flash => matches!(state, BootRom | RemovableDisk),
            Self::Slots | Self::Logs | Self::Actions | Self::Config | Self::Files | Self::Shell => {
                matches!(state, Running | Recovery)
            }
            Self::Telemetry | Self::Status => matches!(state, Running | Recovery | RobotDevice),
            Self::OpenUi => matches!(state, Running | RobotDevice),
            Self::SelfTest | Self::HardwareControl | Self::Install | Self::Gateway => {
                matches!(state, Running)
            }
        }
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

    /// The kinds to show for a device in `state`: `Info` first, then the
    /// granted kinds that apply in that state, in declaration order.
    ///
    /// A granted `Update` is shown as `Recover` in the recovery state, so the
    /// same grant reads as the recovery promise there.
    pub fn shown(&self, state: DeviceState) -> Vec<CapabilityKind> {
        let mut shown = BTreeSet::from([CapabilityKind::Info]);
        for &granted in &self.kinds {
            let kind = match (granted, state) {
                (CapabilityKind::Update, DeviceState::Recovery) => CapabilityKind::Recover,
                _ => granted,
            };
            if kind.applies_in(state) {
                shown.insert(kind);
            }
        }
        shown.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CapabilityKind::*;
    use DeviceState::*;

    fn sample() -> CapabilitySet {
        CapabilitySet::from_kinds([Update, Recover, Logs, Status, Slots, Actions])
    }

    /// Every kind's applicable states, written out by hand. Each match is
    /// exhaustive with no wildcard: a new kind does not compile until its row
    /// is decided here, and `index_of` forces `ALL` to list it.
    fn expected_states(kind: CapabilityKind) -> &'static [DeviceState] {
        match kind {
            Info => &[Running, Recovery, BootRom, RemovableDisk, RobotDevice],
            Update => &[Running],
            Recover => &[Recovery],
            Flash => &[BootRom, RemovableDisk],
            Slots | Logs | Actions | Config | Files | Shell => &[Running, Recovery],
            Telemetry | Status => &[Running, Recovery, RobotDevice],
            OpenUi => &[Running, RobotDevice],
            SelfTest | HardwareControl | Install | Gateway => &[Running],
        }
    }

    fn index_of(kind: CapabilityKind) -> usize {
        match kind {
            Info => 0,
            Update => 1,
            Recover => 2,
            Config => 3,
            Logs => 4,
            Telemetry => 5,
            Actions => 6,
            Gateway => 7,
            OpenUi => 8,
            SelfTest => 9,
            Status => 10,
            HardwareControl => 11,
            Slots => 12,
            Install => 13,
            Files => 14,
            Shell => 15,
            Flash => 16,
        }
    }

    #[test]
    fn applies_in_matches_the_table_for_every_kind_and_state() {
        let states = [Running, Recovery, BootRom, RemovableDisk, RobotDevice];
        for kind in CapabilityKind::ALL {
            for state in states {
                assert_eq!(
                    kind.applies_in(state),
                    expected_states(kind).contains(&state),
                    "{kind:?} in {state:?}"
                );
            }
        }
    }

    #[test]
    fn all_lists_every_kind_once_in_order() {
        assert_eq!(CapabilityKind::ALL.len(), 17);
        for (position, kind) in CapabilityKind::ALL.into_iter().enumerate() {
            assert_eq!(index_of(kind), position, "{kind:?} is out of place in ALL");
        }
    }

    #[test]
    fn info_is_always_first() {
        assert_eq!(CapabilitySet::new().shown(Running), vec![Info]);
        assert_eq!(sample().shown(Running)[0], Info);
    }

    #[test]
    fn running_device_shows_granted_running_kinds() {
        assert_eq!(
            sample().shown(Running),
            vec![Info, Update, Logs, Actions, Status, Slots]
        );
    }

    #[test]
    fn recovery_shows_recover_instead_of_update() {
        assert_eq!(
            sample().shown(Recovery),
            vec![Info, Recover, Logs, Actions, Status, Slots]
        );
    }

    #[test]
    fn boot_rom_and_removable_disk_show_only_info_and_flash() {
        let set = CapabilitySet::from_kinds([Update, Recover, Logs, Flash]);
        assert_eq!(set.shown(BootRom), vec![Info, Flash]);
        assert_eq!(set.shown(RemovableDisk), vec![Info, Flash]);
    }

    #[test]
    fn robot_device_never_shows_writing_kinds() {
        let set = CapabilitySet::from_kinds(CapabilityKind::ALL);
        assert_eq!(
            set.shown(RobotDevice),
            vec![Info, Telemetry, OpenUi, Status]
        );
    }

    #[test]
    fn grants_reports_what_was_granted() {
        let set = sample();
        assert!(set.grants(Logs));
        assert!(!set.grants(Telemetry));
    }
}
