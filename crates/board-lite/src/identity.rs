//! The identity document, read fresh from the system for each answer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// What this board can't find out by itself.
#[derive(Clone, Debug)]
pub struct Options {
    pub model: String,
    /// Its revision, for a board that has them.
    pub rev: Option<String>,
    /// The package version it was built at, `<base>.<count>+<commit>`.
    pub package_version: String,
    /// `/`, or a test's tree.
    pub root: PathBuf,
}

/// What's read from the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub serial: String,
    pub hostname: String,
    pub os_name: Option<String>,
    pub os_version: Option<String>,
    pub macs: BTreeMap<String, String>,
    /// `gaia.slot` and `gaia.trial` on the kernel command line.
    pub slot: Option<(String, bool)>,
}

fn read(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path.trim_start_matches('/'))).ok()
}

/// A `key=value` of os-release, unquoted.
fn os_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix('=')?;
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn macs(root: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(root.join("sys/class/net"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let mac = std::fs::read_to_string(entry.path().join("address")).ok()?;
            let mac = mac.trim().to_string();
            (name != "lo" && !mac.is_empty() && mac != "00:00:00:00:00:00").then_some((name, mac))
        })
        .collect()
}

/// The board's serial, lowercase: its SoC's (Rockchip's kernels put it in
/// cpuinfo), else the device tree's, else its first MAC address.
fn serial(root: &Path, macs: &BTreeMap<String, String>) -> String {
    let clean = |text: &str| {
        let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        (!text.is_empty() && text.chars().any(|c| c != '0')).then(|| text.to_ascii_lowercase())
    };
    read(root, "proc/cpuinfo")
        .and_then(|text| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                if key.trim() == "Serial" {
                    clean(value)
                } else {
                    None
                }
            })
        })
        .or_else(|| read(root, "proc/device-tree/serial-number").and_then(|text| clean(&text)))
        .or_else(|| macs.values().next().map(|mac| mac.replace(':', "")))
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn facts(root: &Path) -> Facts {
    let macs = macs(root);
    let release = read(root, "etc/os-release").unwrap_or_default();
    let cmdline = read(root, "proc/cmdline").unwrap_or_default();
    let word = |prefix: &str| {
        cmdline
            .split_ascii_whitespace()
            .find_map(|word| word.strip_prefix(prefix))
    };
    Facts {
        serial: serial(root, &macs),
        hostname: read(root, "proc/sys/kernel/hostname")
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty() && text != "(none)")
            .unwrap_or_else(|| "board".to_string()),
        os_name: os_field(&release, "ID"),
        os_version: os_field(&release, "VERSION_ID").or_else(|| os_field(&release, "VERSION")),
        slot: word("gaia.slot=").map(|slot| (slot.to_string(), word("gaia.trial=") == Some("1"))),
        macs,
    }
}

pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_or_null(text: Option<&str>) -> String {
    text.map_or_else(|| "null".to_string(), json_string)
}

/// The document (devices/schema/identity.schema.json).
pub fn document(options: &Options, facts: &Facts) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let commit = options
        .package_version
        .split_once('+')
        .map(|(_, commit)| commit);
    let macs: Vec<String> = facts
        .macs
        .iter()
        .map(|(name, mac)| format!("{}:{}", json_string(name), json_string(mac)))
        .collect();
    let slot = facts.slot.as_ref().map_or(String::new(), |(slot, trial)| {
        format!(
            ",\"nand_slot\":{{\"slot\":{},\"trial\":{trial}}}",
            json_string(slot)
        )
    });
    format!(
        "{{\"contract\":1,\"model\":{},\"rev\":{},\"serial\":{},\"hostname\":{},\"os\":{{\"name\":{},\"version\":{}}},\"device_package\":{{\"version\":{},\"commit\":{}}},\"update_methods\":[\"nand-slots\"],\"manage_url\":null,\"macs\":{{{}}},\"time\":{now}{slot}}}",
        json_string(&options.model),
        json_or_null(options.rev.as_deref()),
        json_string(&facts.serial),
        json_string(&facts.hostname),
        json_or_null(facts.os_name.as_deref()),
        json_or_null(facts.os_version.as_deref()),
        json_string(&options.package_version),
        json_or_null(commit),
        macs.join(",")
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A board's tree, as the Luckfox's looks.
    pub fn tree(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("board-lite-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in [
            "proc/sys/kernel",
            "etc",
            "sys/class/net/eth0",
            "sys/class/net/lo",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let write = |path: &str, text: &str| std::fs::write(root.join(path), text).unwrap();
        write(
            "proc/cpuinfo",
            "processor\t: 0\nHardware\t: Generic DT\nSerial\t\t: 8F3A1C2D00112233\n",
        );
        write("proc/sys/kernel/hostname", "luckfox\n");
        write(
            "proc/cmdline",
            "ubi.mtd=5 ubi.block=0,rootfs_b gaia.slot=b gaia.trial=1\n",
        );
        write(
            "etc/os-release",
            "NAME=Buildroot\nID=buildroot\nVERSION_ID=\"2026.02.3\"\n",
        );
        write("sys/class/net/eth0/address", "d2:aa:bb:cc:dd:ee\n");
        write("sys/class/net/lo/address", "00:00:00:00:00:00\n");
        root
    }

    #[test]
    fn the_facts_of_a_luckfox() {
        let root = tree("facts");
        let facts = facts(&root);
        assert_eq!(facts.serial, "8f3a1c2d00112233");
        assert_eq!(facts.hostname, "luckfox");
        assert_eq!(facts.os_name.as_deref(), Some("buildroot"));
        assert_eq!(facts.os_version.as_deref(), Some("2026.02.3"));
        assert_eq!(facts.slot, Some(("b".to_string(), true)));
        assert_eq!(facts.macs.len(), 1, "no loopback");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_serial_falls_back_to_the_device_tree_then_the_mac() {
        let root = tree("serial");
        std::fs::write(root.join("proc/cpuinfo"), "Serial\t: 0000000000000000\n").unwrap();
        assert_eq!(facts(&root).serial, "d2aabbccddee");
        std::fs::create_dir_all(root.join("proc/device-tree")).unwrap();
        std::fs::write(root.join("proc/device-tree/serial-number"), "ABC123\0").unwrap();
        assert_eq!(facts(&root).serial, "abc123");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }
}
