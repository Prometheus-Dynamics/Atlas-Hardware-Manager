//! Claiming the board's disk for a flash, as rpi-imager does: a udev rule
//! that keeps the host's automounter and fsck off that one board's disk for
//! as long as the flash runs, everything of it unmounted before a write,
//! and a refusal while anything still holds it open. A SystemCore mounts
//! (and checks) every USB filesystem it sees; a mount between the write
//! and the read-back changes the partitions.
//!
//! The rule lives under /run and goes when the flash ends, so nothing on
//! the host outlasts the run, and a crash leaves nothing past a boot.

use std::path::{Path, PathBuf};
use std::process::Command;

use atlas_blockdev::Disk;

use crate::flash::{Sink, log};

const RULES_DIR: &str = "/run/udev/rules.d";
/// How long udev may take with the disk's events before the claim goes on.
const SETTLE_TIMEOUT: &str = "--timeout=10";

/// The rule, for as long as it lives.
pub(crate) struct Claim {
    rule: PathBuf,
}

/// The last eight hex digits of a board serial: a boot ROM shows those,
/// the gadget all sixteen. `None` for anything else, so a serial never
/// lands unchecked in a rule.
fn serial_tail(serial: &str) -> Option<String> {
    let serial = serial.trim().to_ascii_lowercase();
    (serial.len() >= 8 && serial.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| serial[serial.len() - 8..].to_string())
}

/// The rule's text: the Pi gadget's disk and partitions, that board's alone
/// when its serial is known, with no filesystem to mount (the SystemCore's
/// automount rule wants one), ignored by UDisks, and not ready for systemd
/// (no mount or fsck units start, and those already bound to it stop).
fn rule_text(tail: Option<&str>) -> String {
    let (board, serial) = match tail {
        Some(tail) => (
            format!("board {tail}"),
            format!("ENV{{ID_SERIAL_SHORT}}==\"*{tail}\", "),
        ),
        None => ("a board".to_string(), String::new()),
    };
    format!(
        "# atlas-usb-agent, while it flashes {board}: no mounts or fsck of its disk.\n\
         ACTION!=\"remove\", SUBSYSTEM==\"block\", ENV{{ID_VENDOR_ID}}==\"0a5c\", \
         ENV{{ID_MODEL_ID}}==\"0104\", {serial}\
         ENV{{ID_FS_USAGE}}=\"\", ENV{{UDISKS_IGNORE}}=\"1\", ENV{{UDISKS_AUTO}}=\"0\", \
         ENV{{SYSTEMD_READY}}=\"0\"\n"
    )
}

fn rule_path(tail: Option<&str>) -> PathBuf {
    Path::new(RULES_DIR).join(format!(
        "98-atlas-usb-flash-{}.rules",
        tail.unwrap_or("any")
    ))
}

impl Claim {
    /// Installs the rule for the board with `serial` (from its ROM or its
    /// gadget), before the board's disk shows.
    pub(crate) fn install(serial: Option<&str>, sink: &Sink) -> Option<Self> {
        let tail = serial.and_then(serial_tail);
        let rule = rule_path(tail.as_deref());
        std::fs::create_dir_all(RULES_DIR).ok()?;
        std::fs::write(&rule, rule_text(tail.as_deref())).ok()?;
        reload_udev();
        log(
            sink,
            format!(
                "kept the automounter and fsck off {} while flashing",
                tail.map_or("the board's disk".to_string(), |tail| format!(
                    "board {tail}'s disk"
                ))
            ),
        );
        Some(Self { rule })
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.rule);
        reload_udev();
    }
}

fn reload_udev() {
    let _ = Command::new("udevadm")
        .args(["control", "--reload"])
        .status();
}

/// `/dev/sda` and its partitions.
fn nodes(disk: &Disk) -> Vec<String> {
    let mut nodes = vec![format!("/dev/{}", disk.name)];
    nodes.extend(
        std::fs::read_dir(Path::new("/sys/block").join(&disk.name))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&disk.name) && name != &disk.name)
            .map(|name| format!("/dev/{name}")),
    );
    nodes
}

/// Device-mapper, md and the like stacked on the disk or a partition.
fn holders(disk: &Disk, nodes: &[String]) -> Vec<String> {
    let sys = Path::new("/sys/block").join(&disk.name);
    nodes
        .iter()
        .filter_map(|node| node.strip_prefix("/dev/"))
        .flat_map(|name| {
            let dir = if name == disk.name {
                sys.join("holders")
            } else {
                sys.join(name).join("holders")
            };
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect()
}

/// Other processes' open files: (pid, command, target).
fn open_files() -> Vec<(u32, String, String)> {
    let own = std::process::id();
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm"))
            .map(|comm| comm.trim().to_string())
            .unwrap_or_default();
        for fd in std::fs::read_dir(entry.path().join("fd"))
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Ok(target) = std::fs::read_link(fd.path()) {
                found.push((pid, comm.clone(), target.to_string_lossy().into_owned()));
            }
        }
    }
    found
}

/// What still uses the disk: mounts of it, holders stacked on it, and
/// processes with it open.
fn users(
    nodes: &[String],
    mounts: &str,
    holders: &[String],
    open: &[(u32, String, String)],
) -> Vec<String> {
    let mut users: Vec<String> = mounts
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let (source, point) = (fields.next()?, fields.next()?);
            nodes
                .iter()
                .any(|node| node == source)
                .then(|| format!("{source} is mounted at {point}"))
        })
        .collect();
    users.extend(
        holders
            .iter()
            .map(|holder| format!("{holder} is stacked on it")),
    );
    users.extend(
        open.iter()
            .filter(|(_, _, target)| nodes.iter().any(|node| node == target))
            .map(|(pid, comm, target)| format!("{comm} (pid {pid}) has {target} open")),
    );
    users
}

/// Takes the disk for writing: unmounts all of it, applies the rule to a
/// disk that showed before it (the automounter had its turn then), and
/// refuses while anything still holds it.
pub(crate) fn take(disk: &Disk, sink: &Sink) -> Result<(), String> {
    let _ = atlas_blockdev::unmount_all(disk);
    let nodes = nodes(disk);
    let _ = Command::new("udevadm")
        .args(["trigger", "--action=change"])
        .args(&nodes)
        .status();
    // Bounded: `trigger --settle` waited forever on a SystemCore once (an
    // event that never finished), and the flash with it.
    let _ = Command::new("udevadm")
        .args(["settle", SETTLE_TIMEOUT])
        .status();
    let _ = atlas_blockdev::unmount_all(disk);
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let users = users(&nodes, &mounts, &holders(disk, &nodes), &open_files());
    if users.is_empty() {
        log(
            sink,
            format!("{} is free: nothing mounts or holds it", disk.name),
        );
        Ok(())
    } else {
        Err(format!(
            "refusing to write {}: {}; nothing was written",
            disk.name,
            users.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_covers_one_board_and_only_its_serial() {
        assert_eq!(serial_tail("A317BCBEE5226D57").as_deref(), Some("e5226d57"));
        assert_eq!(serial_tail("e5226d57").as_deref(), Some("e5226d57"));
        assert_eq!(serial_tail("e5226d5"), None);
        assert_eq!(serial_tail("e5226d57\", RUN+=\"x"), None);
        let rule = rule_text(Some("e5226d57"));
        assert!(rule.contains("ENV{ID_SERIAL_SHORT}==\"*e5226d57\""));
        for off in [
            "ENV{ID_FS_USAGE}=\"\"",
            "ENV{SYSTEMD_READY}=\"0\"",
            "ENV{UDISKS_IGNORE}=\"1\"",
        ] {
            assert!(rule.contains(off), "{off}");
        }
        assert!(rule.contains("ACTION!=\"remove\""));
        // One rule line, and no RUN or anything else that outlives udev's
        // reading of it.
        assert_eq!(
            rule.lines().filter(|line| !line.starts_with('#')).count(),
            1
        );
        assert!(!rule.contains("RUN"));
        assert!(!rule_text(None).contains("ID_SERIAL_SHORT"));
        assert_eq!(
            rule_path(Some("e5226d57")),
            Path::new("/run/udev/rules.d/98-atlas-usb-flash-e5226d57.rules")
        );
    }

    #[test]
    fn a_disk_still_mounted_held_or_open_is_refused() {
        let nodes = vec![
            "/dev/sda".to_string(),
            "/dev/sda1".to_string(),
            "/dev/sda2".to_string(),
        ];
        let mounts = "/dev/mmcblk0p5 / ext4 rw 0 0\n/dev/sda1 /V vfat rw 0 0\n";
        let open = vec![
            (412, "fsck.vfat".to_string(), "/dev/sda2".to_string()),
            (9, "journald".to_string(), "/var/log/x".to_string()),
            (10, "other".to_string(), "/dev/sdb1".to_string()),
        ];
        let users = users(&nodes, mounts, &["dm-0".to_string()], &open);
        assert_eq!(
            users,
            [
                "/dev/sda1 is mounted at /V",
                "dm-0 is stacked on it",
                "fsck.vfat (pid 412) has /dev/sda2 open",
            ]
        );
        assert!(users_of_free_disk(&nodes).is_empty());
    }

    fn users_of_free_disk(nodes: &[String]) -> Vec<String> {
        users(
            nodes,
            "/dev/mmcblk0p5 / ext4 rw 0 0\n/dev/sda10 /x vfat rw 0 0\n",
            &[],
            &[],
        )
    }
}
