//! The names the simulated node answers to: the Lemnos slot and reboot surface (I5), the
//! board's other actions, and its topics. This is the one table to change.
//!
//! I5 names its methods (`system.slots/1`, `system.reboot/1`). Orion needs one action name
//! per method, so the sim names each `system.<contract>.<method>` with `-` for `_`. How
//! Orion carries Lemnos methods to clients is OPEN (see `testing/sim-node/README.md`); a
//! change there is an edit to the constants below.
//!
//! The board's other actions (`locate`, `self-test`, `set-clock`, `support-bundle`,
//! `data.*`, `fs.*`, `status`) are not in I5. They stay here as provisional names for the
//! C-series briefs that name them.

/// One method of a Lemnos contract and the name the node answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Method {
    /// The contract tag, for example `system.slots/1`.
    pub contract: &'static str,
    /// The method in the contract.
    pub lemnos: &'static str,
    /// The Orion action name the node registers.
    pub action: &'static str,
    /// Whether the method is a long job (start, progress, done, cancel).
    pub long_job: bool,
}

/// The I5 surface: every method of `system.slots/1` and `system.reboot/1`.
pub const SURFACE: &[Method] = &[
    Method {
        contract: "system.slots/1",
        lemnos: "list",
        action: SLOTS_LIST,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "install",
        action: SLOTS_INSTALL,
        long_job: true,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "write",
        action: SLOTS_WRITE,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "set_default",
        action: SLOTS_SET_DEFAULT,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "trial",
        action: SLOTS_TRIAL,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "confirm",
        action: SLOTS_CONFIRM,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "remove",
        action: SLOTS_REMOVE,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "pin",
        action: SLOTS_PIN,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "set_keep",
        action: SLOTS_SET_KEEP,
        long_job: false,
    },
    Method {
        contract: "system.reboot/1",
        lemnos: "reboot",
        action: REBOOT,
        long_job: false,
    },
];

/// Actions the simulated node registers: the I5 surface, then the board's own.
pub const ACTIONS: &[&str] = &[
    SLOTS_LIST,
    SLOTS_INSTALL,
    SLOTS_WRITE,
    SLOTS_SET_DEFAULT,
    SLOTS_TRIAL,
    SLOTS_CONFIRM,
    SLOTS_REMOVE,
    SLOTS_PIN,
    SLOTS_SET_KEEP,
    REBOOT,
    STATUS,
    LOCATE,
    SELF_TEST,
    SET_CLOCK,
    SUPPORT_BUNDLE,
    DATA_BACKUP,
    DATA_RESTORE,
    FS_LIST,
    FS_READ,
    FS_WRITE,
    FS_REMOVE,
];

/// Topics the simulated node publishes.
pub const TOPICS: &[&str] = &[READINGS, HARDWARE, LOGS];

// system.slots/1. Arguments are `key=value` lines (OPEN: Orion's encoding of Lemnos args).
// `install` takes `size` (u64 LE, 8 bytes), `pin` and `keep_old` (one byte each, 0 or 1).
// `write` takes `offset` (u64 LE, 8 bytes) then the chunk bytes.
pub const SLOTS_LIST: &str = "system.slots.list";
pub const SLOTS_INSTALL: &str = "system.slots.install";
pub const SLOTS_WRITE: &str = "system.slots.write";
pub const SLOTS_SET_DEFAULT: &str = "system.slots.set-default";
pub const SLOTS_TRIAL: &str = "system.slots.trial";
pub const SLOTS_CONFIRM: &str = "system.slots.confirm";
pub const SLOTS_REMOVE: &str = "system.slots.remove";
pub const SLOTS_PIN: &str = "system.slots.pin";
pub const SLOTS_SET_KEEP: &str = "system.slots.set-keep";

// system.reboot/1.
pub const REBOOT: &str = "system.reboot.reboot";

// The board's own actions, outside I5 (provisional).
/// The boot and slot state as text.
pub const STATUS: &str = "status";
pub const LOCATE: &str = "locate";
pub const SELF_TEST: &str = "self-test";
pub const SET_CLOCK: &str = "set-clock";
/// A small text report of the board's state, returned as output.
pub const SUPPORT_BUNDLE: &str = "support-bundle";
// The data partition. `data.restore` takes a backup as its args; `fs.*` take `path=`.
pub const DATA_BACKUP: &str = "data.backup";
pub const DATA_RESTORE: &str = "data.restore";
pub const FS_LIST: &str = "fs.list";
pub const FS_READ: &str = "fs.read";
pub const FS_WRITE: &str = "fs.write";
pub const FS_REMOVE: &str = "fs.remove";

/// Live readings: temperature (C), input voltage (V), fan speed (rpm).
pub const READINGS: &str = "readings";
/// The hardware description, once at start.
pub const HARDWARE: &str = "hardware";
/// Log lines as they are written.
pub const LOGS: &str = "logs";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for name in ACTIONS.iter().chain(TOPICS) {
            assert!(seen.insert(*name), "{name} is listed twice");
            assert!(!name.is_empty() && !name.contains(' '));
        }
    }

    #[test]
    fn every_surface_method_is_registered() {
        for method in SURFACE {
            assert!(
                ACTIONS.contains(&method.action),
                "{} is not registered",
                method.action
            );
        }
        assert_eq!(SURFACE.iter().filter(|method| method.long_job).count(), 1);
    }
}
