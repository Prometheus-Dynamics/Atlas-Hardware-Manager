//! What Atlas and `atlas-usb-agent` say to each other: the agent's HTTP API
//! (`atlas-usb-agent serve`, on [`PORT`]) and the events a flash reports.
//!
//! The agent runs on another Linux host that boards in USB boot are plugged
//! into (a SystemCore): it lists the USB devices there, takes boot files
//! and images, and runs flashes as jobs that go on when Atlas's link drops.
//!
//! ```text
//! GET    /v1/agent              AgentInfo
//! GET    /v1/usb                [UsbDevice]
//! PUT    /v1/boot/<file>        the mass-storage boot files (BOOT_FILES)
//! PUT    /v1/images/<name>      an image; x-sha256 must match (Stored)
//! POST   /v1/flash              FlashRequest -> JobStarted (409 while one runs)
//! GET    /v1/jobs/<id>?from=N   JobEvents: events from N on
//! DELETE /v1/jobs/<id>          cancel
//! ```

use std::collections::BTreeMap;

use atlas_blockdev::WriteProgress;
use serde::{Deserialize, Serialize};

/// The agent's port: in FRC's team range (5800-5810), so the field passes
/// it, and free on a SystemCore.
pub const PORT: u16 = 5809;

/// The mass-storage gadget's files the agent boots boards with.
pub const BOOT_FILES: [&str; 3] = ["bootfiles.bin", "boot.img", "config.txt"];

/// The header carrying an upload's SHA-256.
pub const SHA256_HEADER: &str = "x-sha256";

/// What a flash reports, in order; the last is `done` or `error`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    Log {
        text: String,
        /// The agent's resident memory then, in KiB.
        #[serde(default)]
        rss_kib: Option<u64>,
    },
    Boot {
        /// 0 to 1 through USB boot.
        progress: f32,
    },
    Booted {
        serial: Option<String>,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
    },
    Disk {
        name: String,
        size_bytes: u64,
    },
    Write(WriteProgress),
    Written {
        bytes: u64,
        seconds: u64,
        sha256: String,
    },
    Done {
        /// The console the restart went through, or why it didn't.
        restarted: Option<String>,
        restart_error: Option<String>,
    },
    Error {
        message: String,
    },
}

impl Event {
    pub fn is_last(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub version: String,
    /// Whether it holds the boot files.
    pub boot_files: bool,
    /// The job running now.
    pub running: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbKind {
    /// A Raspberry Pi in USB boot (its boot ROM).
    UsbBoot,
    /// A Raspberry Pi showing its eMMC (the mass-storage gadget).
    Gadget,
    Other,
}

/// A USB device on the agent's host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbDevice {
    /// sysfs form: `3-1.2`.
    pub port: String,
    pub vendor: String,
    pub product: String,
    pub name: Option<String>,
    pub serial: Option<String>,
    pub kind: UsbKind,
}

impl UsbDevice {
    pub fn kind_of(vendor: &str, product: &str) -> UsbKind {
        match (vendor, product) {
            ("0a5c", "2711" | "2712" | "2763" | "2764") => UsbKind::UsbBoot,
            ("0a5c", "0104") => UsbKind::Gadget,
            _ => UsbKind::Other,
        }
    }
}

/// An image the agent stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashRequest {
    /// The board's port (sysfs form).
    pub port: String,
    /// A stored image's name.
    pub image: String,
    pub sha256: Option<String>,
    /// Files to put on the boot partition after the write, by path there
    /// (`board/authorized_keys`).
    #[serde(default)]
    pub boot_writes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStarted {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobEvents {
    pub id: String,
    pub events: Vec<Event>,
    /// Ask from here next.
    pub next: usize,
    pub finished: bool,
}

/// How Atlas addresses a board behind an agent: the agent's `host:port`
/// and the board's USB port there, `usb-agent://172.27.0.1:5809/3-1.2`.
pub const ADDRESS_SCHEME: &str = "usb-agent://";

pub fn board_address(agent: &str, usb_port: &str) -> String {
    format!("{ADDRESS_SCHEME}{agent}/{usb_port}")
}

/// The agent's `host:port` and the board's USB port, from a
/// [`board_address`].
pub fn parse_board_address(address: &str) -> Option<(&str, &str)> {
    let (agent, usb_port) = address.strip_prefix(ADDRESS_SCHEME)?.rsplit_once('/')?;
    (!agent.is_empty() && valid_port(usb_port)).then_some((agent, usb_port))
}

/// `3-1.2`: a bus number, then dot-separated port numbers.
pub fn valid_port(port: &str) -> bool {
    port.split_once('-').is_some_and(|(bus, ports)| {
        !bus.is_empty()
            && bus.chars().all(|c| c.is_ascii_digit())
            && !ports.is_empty()
            && ports
                .split('.')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    })
}

/// A stored file's name: letters, digits, `.`, `-`, `_`, `+`, not leading
/// with a dot, at most 200.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_and_names_are_checked() {
        assert!(valid_port("3-1.1"));
        assert!(valid_port("1-4"));
        assert!(!valid_port("3-"));
        assert!(!valid_port("3-1..1"));
        assert!(!valid_port("../3-1"));
        assert!(valid_name("helios-2026.2.0-dev.801+ab.img.xz"));
        assert!(!valid_name("../x"));
        assert!(!valid_name(".hidden"));
        assert!(!valid_name("a/b"));
        let address = board_address("172.27.0.1:5809", "3-1.1.4.1");
        assert_eq!(address, "usb-agent://172.27.0.1:5809/3-1.1.4.1");
        assert_eq!(
            parse_board_address(&address),
            Some(("172.27.0.1:5809", "3-1.1.4.1"))
        );
        assert_eq!(parse_board_address("usb-agent://h:1/x"), None);
        assert_eq!(parse_board_address("001-2"), None);
    }

    #[test]
    fn events_round_trip_as_json_lines() {
        let events = [
            Event::Write(WriteProgress::Writing {
                input_done: 1,
                input_total: 2,
                written: 3,
            }),
            Event::Log {
                text: "x".into(),
                rss_kib: Some(5),
            },
            Event::Done {
                restarted: Some("ttyACM0".into()),
                restart_error: None,
            },
        ];
        for event in events {
            let line = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), event);
        }
        let line = r#"{"event":"write","phase":"verifying","done":1,"total":2}"#;
        assert!(matches!(
            serde_json::from_str::<Event>(line).unwrap(),
            Event::Write(WriteProgress::Verifying { done: 1, total: 2 })
        ));
        assert_eq!(UsbDevice::kind_of("0a5c", "2712"), UsbKind::UsbBoot);
        assert_eq!(UsbDevice::kind_of("0a5c", "0104"), UsbKind::Gadget);
    }
}
