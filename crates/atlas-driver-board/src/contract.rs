//! The identity a board reports, and how it becomes an Atlas identity.

use std::collections::BTreeMap;

use atlas_devices::DevicePackage;
use atlas_driver::{DeviceAction, DeviceKey, DeviceMode, Identity, LinkId};
use serde::Deserialize;

pub const IDENTITY_PATH: &str = "/.well-known/pd-device";

/// A board clock this far off (seconds) is reported, and can be set.
pub const CLOCK_TOLERANCE_S: i64 = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct NameVersion {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
}

/// `GET /.well-known/pd-device`. Unknown fields are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct BoardIdentity {
    #[serde(default)]
    pub contract: u32,
    pub model: String,
    #[serde(default)]
    pub rev: Option<String>,
    pub serial: String,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub os: NameVersion,
    #[serde(default)]
    pub device_package: NameVersion,
    #[serde(default)]
    pub bootloader: NameVersion,
    #[serde(default)]
    pub update_methods: Vec<String>,
    #[serde(default)]
    pub manage_url: Option<String>,
    #[serde(default)]
    pub macs: BTreeMap<String, String>,
    /// Optional live endpoints by name: `metrics`, `logs`, `actions`.
    #[serde(default)]
    pub endpoints: BTreeMap<String, String>,
    /// Actions the `actions` endpoint accepts.
    #[serde(default)]
    pub actions: Vec<ReportedAction>,
    /// A camera view: an MJPEG stream or a still image, path or URL.
    #[serde(default)]
    pub camera_stream: Option<String>,
    /// The A/B updater's state (`update status`), when the board has one.
    #[serde(default)]
    pub update: Option<UpdateReport>,
    /// The board's clock (Unix seconds) when it answered.
    #[serde(default)]
    pub time: Option<i64>,
    /// Root-only diagnostics run over SSH, such as `selftest`.
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// `update status` as the identity carries it; every field is optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct UpdateReport {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub slot_active: Option<String>,
    #[serde(default)]
    pub version_active: Option<String>,
    #[serde(default)]
    pub version_staged: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// An action as a device lists it; only `id` is required.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ReportedAction {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub destructive: bool,
}

impl ReportedAction {
    pub fn to_action(&self) -> DeviceAction {
        DeviceAction {
            id: self.id.clone(),
            label: self.label.clone().unwrap_or_else(|| self.id.clone()),
            destructive: self.destructive,
        }
    }
}

impl BoardIdentity {
    /// Builds an identity from the mDNS TXT record alone, for when the
    /// identity endpoint does not answer.
    pub fn from_txt(txt: &BTreeMap<String, String>) -> Option<Self> {
        let get = |key: &str| {
            txt.get(key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Some(Self {
            contract: get("contract")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            model: get("model")?,
            rev: get("rev"),
            serial: get("serial")?,
            os: NameVersion {
                name: get("os"),
                version: get("os_ver"),
                commit: None,
            },
            ..Self::default()
        })
    }

    /// The Atlas identity: keyed by model and serial, with the OS and the
    /// rest as attributes.
    pub fn to_identity(
        &self,
        package: Option<&DevicePackage>,
        link: LinkId,
        address: String,
    ) -> Identity {
        let model_name = package
            .map(|package| package.manifest.display_name().to_string())
            .unwrap_or_else(|| self.model.clone());
        let mut versions = BTreeMap::new();
        if let Some(version) = &self.os.version {
            versions.insert("os".to_string(), version.clone());
        }
        if let Some(version) = &self.bootloader.version {
            versions.insert("bootloader".to_string(), version.clone());
        }
        if let Some(version) = &self.device_package.version {
            versions.insert("device_package".to_string(), version.clone());
        }

        let mut attributes = BTreeMap::new();
        let mut put = |key: &str, value: &Option<String>| {
            if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
                attributes.insert(key.to_string(), value.clone());
            }
        };
        put("os", &self.os.name);
        put("os_version", &self.os.version);
        put("revision", &self.rev);
        put("hostname", &self.hostname);
        put("manage_url", &self.manage_url);
        put("device_package_commit", &self.device_package.commit);
        // How far the board's clock is from this computer's, in seconds
        // (negative: behind). A Raze has no RTC battery.
        if let (Some(time), Ok(now)) = (
            self.time,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH),
        ) {
            let offset = time - now.as_secs() as i64;
            // Only when it's off, rounded so it stays put between scans.
            if offset.abs() >= CLOCK_TOLERANCE_S {
                let rounded = (offset as f64 / 10.0).round() as i64 * 10;
                put(
                    atlas_driver::attributes::CLOCK_OFFSET_S,
                    &Some(rounded.to_string()),
                );
            }
        }
        if let Some(update) = &self.update {
            put("update_state", &update.state);
            put("slot_active", &update.slot_active);
            put("update_version_active", &update.version_active);
            put("update_version_staged", &update.version_staged);
            put("update_error", &update.error);
        }
        if !self.update_methods.is_empty() {
            attributes.insert("update_methods".into(), self.update_methods.join(", "));
        }
        for (interface, mac) in &self.macs {
            attributes.insert(format!("mac.{interface}"), mac.clone());
        }
        attributes.insert("contract".into(), self.contract.to_string());
        if let Some(board) = atlas_driver::attributes::normalize_board_serial(&self.serial) {
            attributes.insert(atlas_driver::attributes::BOARD_SERIAL.into(), board);
        }
        if let Some(stream) = self
            .camera_stream
            .as_deref()
            .and_then(|stream| crate::live::resolve(&address, stream))
        {
            attributes.insert(atlas_driver::attributes::CAMERA_STREAM.into(), stream);
        }

        Identity {
            key: DeviceKey::new(
                self.model.to_ascii_lowercase(),
                self.serial.to_ascii_lowercase(),
            ),
            model: match &self.rev {
                Some(rev) => {
                    let label = package
                        .map(|package| package.manifest.revision_label(rev))
                        .unwrap_or_else(|| rev.clone());
                    format!("{model_name} {label}")
                }
                None => model_name,
            },
            mode: DeviceMode::Normal,
            versions,
            name: self.hostname.clone(),
            link,
            address,
            attributes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "contract": 1, "model": "raze", "rev": "b",
        "serial": "10000000ABCDEF01", "hostname": "raze-abcdef01",
        "os": { "name": "photonvision", "version": "v2026.1.0" },
        "device_package": { "version": "0.3.0", "commit": "abc123" },
        "bootloader": { "version": "2025-06-01" },
        "update_methods": ["image-write"],
        "manage_url": "http://raze-abcdef01.local:5800/",
        "macs": { "usb0": "02:aa:bb:cc:dd:01" },
        "update": { "state": "confirmed", "slot_active": "B", "slot_staged": "B",
                    "version_active": "v2026.1.0", "version_staged": "v2026.1.0",
                    "progress": 1000, "error": "" },
        "something_new": 42
    }"#;

    #[test]
    fn identity_json_maps_to_model_serial_and_attributes() {
        let reported: BoardIdentity = serde_json::from_str(SAMPLE).unwrap();
        let identity = reported.to_identity(None, LinkId("mdns".into()), "x".into());

        assert_eq!(identity.key, DeviceKey::new("raze", "10000000abcdef01"));
        assert_eq!(identity.model, "raze b");
        assert_eq!(identity.primary_version(), Some("v2026.1.0"));
        assert_eq!(identity.name.as_deref(), Some("raze-abcdef01"));
        assert_eq!(identity.attributes["os"], "photonvision");
        assert_eq!(identity.attributes["mac.usb0"], "02:aa:bb:cc:dd:01");
        assert_eq!(identity.versions["bootloader"], "2025-06-01");
        assert_eq!(identity.attributes["board_serial"], "abcdef01");
        assert_eq!(identity.attributes["update_state"], "confirmed");
        assert_eq!(identity.attributes["slot_active"], "B");
        assert!(!identity.attributes.contains_key("update_error"));
    }

    #[test]
    fn txt_records_are_enough_when_the_endpoint_is_down() {
        let txt: BTreeMap<String, String> = [
            ("contract", "1"),
            ("model", "raze"),
            ("serial", "abc123"),
            ("os", "helios"),
            ("os_ver", "2026.3.1"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let reported = BoardIdentity::from_txt(&txt).unwrap();
        assert_eq!(reported.os.version.as_deref(), Some("2026.3.1"));

        let mut no_serial = txt.clone();
        no_serial.remove("serial");
        assert!(BoardIdentity::from_txt(&no_serial).is_none());
    }
}
