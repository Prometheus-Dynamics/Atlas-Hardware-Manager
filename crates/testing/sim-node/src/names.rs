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
        lemnos: "accept",
        action: SLOTS_ACCEPT,
        long_job: false,
    },
    Method {
        contract: "system.slots/1",
        lemnos: "install_file",
        action: SLOTS_INSTALL_FILE,
        long_job: true,
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
    SLOTS_ACCEPT,
    SLOTS_INSTALL_FILE,
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

// The hello facts, as Lemnos P1c defines them (Lemnos docs/identity.md, the bridge's `hello.*`
// topics). One text field named `value`, history 2, at most 256 bytes, UTF-8, no trailing newline.
// Each fact has one owner, and the simulated node publishes what that owner publishes:
// - Lemnos (the bridge, from `system.identity/1` and the daemon): `family`, `serial`, `model`,
//   `version.slot`, `version.product`, `version.os`, `version.lemnos`, and `offers` (the contract
//   ids of the devices running now). Atlas's board driver reads these.
// - Orion (its own part, not Lemnos's): `state` (`running`, `recovery`, or `flasher`, which a
//   flasher image's program sets). Orion's recovery build publishes it; the simulated node keeps
//   it as the mode it was started in. Atlas reads it with the hello document.
// `name` and `revision` are fields of Orion's hello document, not facts, so there is no topic
// for them: the simulated node has none.
pub const HELLO_FAMILY: &str = "hello.family";
pub const HELLO_SERIAL: &str = "hello.serial";
pub const HELLO_MODEL: &str = "hello.model";
pub const HELLO_VERSION_SLOT: &str = "hello.version.slot";
pub const HELLO_VERSION_PRODUCT: &str = "hello.version.product";
pub const HELLO_VERSION_OS: &str = "hello.version.os";
pub const HELLO_VERSION_LEMNOS: &str = "hello.version.lemnos";
pub const HELLO_OFFERS: &str = "hello.offers";
/// Owned by Orion, not Lemnos. See above.
pub const HELLO_STATE: &str = "hello.state";

/// Every hello topic the node may publish.
pub const HELLO: &[&str] = &[
    HELLO_FAMILY,
    HELLO_SERIAL,
    HELLO_MODEL,
    HELLO_VERSION_SLOT,
    HELLO_VERSION_PRODUCT,
    HELLO_VERSION_OS,
    HELLO_VERSION_LEMNOS,
    HELLO_OFFERS,
    HELLO_STATE,
];

// system.slots/1. Arguments are UTF-8 words, separated by spaces, as Lemnos's bridge takes them
// (`lemnos_wire::text::encode_args`): numbers in decimal, bools as true/false (or on/off, 1/0),
// strings as one word. A bundle never goes as an argument: `accept <size>` checks there is room,
// the bundle is pushed as a file (Orion's `Files::push`) into `INCOMING`, and
// `install_file <name> <pin> <keep_old>` installs it as a long job. Direct socket callers
// (`install` + `write`) are not on the Orion surface.
pub const SLOTS_LIST: &str = "system.slots.list";
pub const SLOTS_ACCEPT: &str = "system.slots.accept";
pub const SLOTS_INSTALL_FILE: &str = "system.slots.install-file";
pub const SLOTS_SET_DEFAULT: &str = "system.slots.set-default";
pub const SLOTS_TRIAL: &str = "system.slots.trial";
pub const SLOTS_CONFIRM: &str = "system.slots.confirm";
pub const SLOTS_REMOVE: &str = "system.slots.remove";
pub const SLOTS_PIN: &str = "system.slots.pin";
pub const SLOTS_SET_KEEP: &str = "system.slots.set-keep";

/// The directory a pushed bundle lands in, relative to the data partition. The driver owns it:
/// it is cleared when the node starts, and a file is removed when its install job ends.
pub const INCOMING: &str = "slots/.incoming/";

/// Whether `name` is one of Lemnos's own actions (the slot and reboot surface).
#[must_use]
pub fn is_lemnos(name: &str) -> bool {
    name.starts_with("system.")
}

/// Whether `name` is one path component of `[A-Za-z0-9._-]`, not `.` or `..`.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

// system.reboot/1.
pub const REBOOT: &str = "system.reboot.reboot";

// The board's own actions, outside I5 (provisional). Their words are the same: spaces between
// them, and bytes as hex.
/// The boot and slot state as text.
pub const STATUS: &str = "status";
pub const LOCATE: &str = "locate";
pub const SELF_TEST: &str = "self-test";
pub const SET_CLOCK: &str = "set-clock";
/// A small text report of the board's state, returned as output.
pub const SUPPORT_BUNDLE: &str = "support-bundle";
// The data partition. `data.backup` answers the partition as hex; `data.restore` takes that hex;
// `fs.*` take a path as a word.
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
        for name in ACTIONS.iter().chain(TOPICS).chain(HELLO) {
            assert!(seen.insert(*name), "{name} is listed twice");
            assert!(!name.is_empty() && !name.contains(' '));
        }
    }

    #[test]
    fn hello_topics_are_named_hello_dot_fact() {
        for topic in HELLO {
            assert!(topic.starts_with("hello."), "{topic}");
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
