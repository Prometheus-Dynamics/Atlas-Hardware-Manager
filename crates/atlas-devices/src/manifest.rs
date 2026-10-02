use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// A USB id written as `"0a5c"`, `"0x0a5c"`, or a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct UsbId(pub u16);

impl<'de> Deserialize<'de> for UsbId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let parsed = match &value {
            Value::Number(number) => number.as_u64().and_then(|n| u16::try_from(n).ok()),
            Value::String(text) => {
                let hex = text
                    .trim()
                    .trim_start_matches("0x")
                    .trim_start_matches("0X");
                u16::from_str_radix(hex, 16).ok()
            }
            _ => None,
        };
        parsed
            .map(UsbId)
            .ok_or_else(|| serde::de::Error::custom(format!("not a USB id: {value}")))
    }
}

/// One hardware revision and how to recognise it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub id: String,
    /// A value, a list of values, or an object of `fact: value` pairs that
    /// must all match, compared against facts read from the board (for
    /// example `USER_BOARDREV` from the bootloader's OTP metadata).
    #[serde(default, rename = "match")]
    pub matches: Option<Value>,
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn value_matches(rule: &Value, facts: &BTreeMap<String, String>) -> bool {
    match rule {
        Value::String(text) => facts.values().any(|fact| same(fact, text)),
        Value::Array(items) => items.iter().any(|item| value_matches(item, facts)),
        Value::Object(pairs) => {
            !pairs.is_empty()
                && pairs.iter().all(|(key, expected)| {
                    let actual = facts
                        .iter()
                        .find(|(name, _)| same(name, key))
                        .map(|(_, value)| value.as_str());
                    match (actual, expected) {
                        (Some(actual), Value::String(expected)) => same(actual, expected),
                        (Some(actual), Value::Array(options)) => options
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|option| same(actual, option)),
                        _ => false,
                    }
                })
        }
        _ => false,
    }
}

impl Revision {
    pub fn matches(&self, facts: &BTreeMap<String, String>) -> bool {
        !self.is_default()
            && self
                .matches
                .as_ref()
                .is_some_and(|rule| value_matches(rule, facts))
    }

    /// `"match": { "default": true }`: used when no other revision matches.
    pub fn is_default(&self) -> bool {
        self.matches
            .as_ref()
            .and_then(|rule| rule.get("default"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// USB descriptor strings a running device presents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbGadget {
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub vid: Option<UsbId>,
    #[serde(default)]
    pub pid: Option<UsbId>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IdentityRules {
    /// Where the OS reads the serial; free-form, for the OS builds.
    #[serde(default)]
    pub serial_source: Option<Value>,
    /// For example `raze-{serial8}`.
    #[serde(default)]
    pub hostname_pattern: Option<String>,
    #[serde(default)]
    pub mac_rule: Option<Value>,
    #[serde(default)]
    pub usb_gadget: Option<UsbGadget>,
    /// Port of the device package's identity responder, when fixed.
    #[serde(default)]
    pub identity_port: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbBootId {
    pub vid: UsbId,
    pub pid: UsbId,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recovery {
    #[serde(default)]
    pub usb_boot: Option<UsbBootId>,
    /// For example `mass-storage-gadget`.
    #[serde(default)]
    pub method: Option<String>,
    /// `emmc` or `nvme`.
    #[serde(default)]
    pub storage_target: Option<String>,
    /// Short steps shown while Atlas waits for the board in USB boot.
    #[serde(default)]
    pub instructions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EepromFile {
    pub path: String,
    /// `pieeprom`, `pieeprom-sig`, `recovery`, `bootconf`, ...
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EepromInfo {
    /// A version string, or an object such as `{ "date", "timestamp", "hash" }`.
    #[serde(default)]
    pub min_version: Option<Value>,
    #[serde(default)]
    pub files: Vec<EepromFile>,
    /// For example `rpiboot-recovery`.
    #[serde(default)]
    pub update_method: Option<String>,
}

/// `devices/<model>/manifest.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceManifest {
    #[serde(default)]
    pub schema: Option<String>,
    pub contract: u32,
    pub model: String,
    #[serde(default)]
    pub package_version: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub revisions: Vec<Revision>,
    #[serde(default)]
    pub rev_source: Option<Value>,
    #[serde(default)]
    pub identity: IdentityRules,
    #[serde(default)]
    pub recovery: Recovery,
    #[serde(default)]
    pub eeprom: Option<EepromInfo>,
    /// A list of names, or an object keyed by capability name.
    #[serde(default)]
    pub capabilities: Value,
}

impl DeviceManifest {
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.model)
    }

    /// The revision whose rule matches these board facts if exactly one
    /// does, else the default revision when there is one.
    pub fn revision_for(&self, facts: &BTreeMap<String, String>) -> Option<&Revision> {
        let mut matching = self.revisions.iter().filter(|rev| rev.matches(facts));
        match (matching.next(), matching.next()) {
            (Some(only), None) => Some(only),
            (None, _) => self.revisions.iter().find(|rev| rev.is_default()),
            (Some(_), Some(_)) => None,
        }
    }

    /// Whether a running USB device's strings identify this model.
    pub fn matches_gadget(&self, manufacturer: Option<&str>, product: Option<&str>) -> bool {
        let Some(gadget) = &self.identity.usb_gadget else {
            return false;
        };
        let check = |want: &Option<String>, have: Option<&str>| match (want, have) {
            (Some(want), Some(have)) => same(want, have),
            (Some(_), None) => false,
            (None, _) => true,
        };
        (gadget.manufacturer.is_some() || gadget.product.is_some())
            && check(&gadget.manufacturer, manufacturer)
            && check(&gadget.product, product)
    }

    pub fn has_capability(&self, name: &str) -> bool {
        match &self.capabilities {
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .any(|cap| same(cap, name)),
            Value::Object(map) => map.keys().any(|cap| same(cap, name)),
            _ => false,
        }
    }

    /// The EEPROM file with this role (`pieeprom`, `pieeprom-sig`, `recovery`).
    pub fn eeprom_file(&self, role: &str) -> Option<&EepromFile> {
        self.eeprom
            .as_ref()?
            .files
            .iter()
            .find(|file| file.role.as_deref().is_some_and(|r| same(r, role)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const RAZE: &str = r#"{
        "schema": "https://example.invalid/device-manifest.json",
        "contract": 1,
        "model": "raze",
        "package_version": "0.1.0",
        "display_name": "Raze",
        "revisions": [
            { "id": "a", "match": { "USER_BOARDREV": "d04170" } },
            { "id": "b", "match": ["d04171", "d04172"] }
        ],
        "identity": {
            "hostname_pattern": "raze-{serial8}",
            "usb_gadget": { "manufacturer": "Prometheus Dynamics", "product": "Raze", "vid": "0x1d6b", "pid": 260 }
        },
        "recovery": {
            "usb_boot": { "vid": "0a5c", "pid": "2712" },
            "method": "mass-storage-gadget",
            "storage_target": "emmc",
            "instructions": ["Hold BOOT while connecting USB-C"]
        },
        "eeprom": { "min_version": "2025-06-01", "files": [{ "path": "eeprom/pieeprom.bin" }] },
        "capabilities": ["camera", "fan"],
        "future_field": { "ignored": true }
    }"#;

    fn facts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn manifests_parse_leniently() {
        let manifest: DeviceManifest = serde_json::from_str(RAZE).unwrap();
        assert_eq!(manifest.model, "raze");
        assert_eq!(manifest.recovery.usb_boot.unwrap().pid, UsbId(0x2712));
        let gadget = manifest.identity.usb_gadget.as_ref().unwrap();
        assert_eq!(gadget.vid, Some(UsbId(0x1d6b)));
        assert_eq!(gadget.pid, Some(UsbId(260)));
        assert!(manifest.has_capability("FAN"));

        let minimal: DeviceManifest =
            serde_json::from_str(r#"{ "contract": 1, "model": "x" }"#).unwrap();
        assert_eq!(minimal.display_name(), "x");
    }

    #[test]
    fn revisions_match_objects_and_lists() {
        let manifest: DeviceManifest = serde_json::from_str(RAZE).unwrap();
        let a = facts(&[("USER_BOARDREV", "D04170"), ("MAC_ADDR", "x")]);
        let b = facts(&[("user_boardrev", "d04172")]);
        let none = facts(&[("USER_BOARDREV", "c03141")]);

        assert_eq!(manifest.revision_for(&a).map(|r| r.id.as_str()), Some("a"));
        assert_eq!(manifest.revision_for(&b).map(|r| r.id.as_str()), Some("b"));
        assert!(manifest.revision_for(&none).is_none());

        let with_default: DeviceManifest = serde_json::from_str(
            r#"{ "contract": 1, "model": "x",
                 "revisions": [{ "id": "a", "match": { "default": true } }],
                 "capabilities": { "fan": {}, "camera": {} } }"#,
        )
        .unwrap();
        assert_eq!(
            with_default.revision_for(&none).map(|r| r.id.as_str()),
            Some("a")
        );
        assert!(with_default.has_capability("fan"));
    }

    #[test]
    fn gadget_strings_identify_the_model() {
        let manifest: DeviceManifest = serde_json::from_str(RAZE).unwrap();
        assert!(manifest.matches_gadget(Some("prometheus dynamics"), Some("Raze")));
        assert!(!manifest.matches_gadget(Some("Linux Foundation"), Some("Raze")));
        assert!(!manifest.matches_gadget(None, None));
    }
}
