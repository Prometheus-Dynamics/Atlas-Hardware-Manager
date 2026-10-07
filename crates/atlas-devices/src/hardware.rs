//! Typed views of a manifest's hardware sections (`capabilities.leds`,
//! `fan`, `i2c`, `selftest`). Every field is optional and parsing is lenient,
//! so older manifests, which had fewer facts and wrote I2C addresses as
//! `"0x18"` strings, still read.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::DeviceManifest;

/// A 7-bit I2C address, written as a number or as `"0x18"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct I2cAddress(pub u8);

impl<'de> Deserialize<'de> for I2cAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let parsed = match &value {
            Value::Number(number) => number.as_u64().and_then(|n| u8::try_from(n).ok()),
            Value::String(text) => {
                let text = text.trim();
                match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                    Some(hex) => u8::from_str_radix(hex, 16).ok(),
                    None => text.parse().ok(),
                }
            }
            _ => None,
        };
        parsed
            .filter(|address| *address <= 0x7f)
            .map(I2cAddress)
            .ok_or_else(|| serde::de::Error::custom(format!("not an I2C address: {value}")))
    }
}

/// Who confirmed a fact, when and how; `None` in [`LedRing::verified`] and
/// friends means the manifest says "unverified".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub by: String,
    pub date: String,
    pub method: String,
}

fn verified_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, Option<Verification>>, D::Error> {
    let raw = std::collections::BTreeMap::<String, Value>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(fact, note)| (fact, serde_json::from_value(note).ok()))
        .collect())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedIndex {
    /// The driver slot of LED index 0.
    #[serde(default)]
    pub offset: u32,
    /// 1, or -1 when indexes run the other way round.
    #[serde(default = "one")]
    pub direction: i32,
}

fn one() -> i32 {
    1
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedRing {
    #[serde(default)]
    pub part: Option<String>,
    #[serde(default)]
    pub gpio: Option<u32>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub device: Option<String>,
    /// `grb24`, `rgb24`, `grbw32` or `rgbw32`.
    #[serde(default)]
    pub wire_format: Option<String>,
    #[serde(default)]
    pub index: Option<LedIndex>,
    #[serde(default, deserialize_with = "verified_map")]
    pub verified: std::collections::BTreeMap<String, Option<Verification>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanPwm {
    #[serde(default)]
    pub controller: Option<String>,
    #[serde(default)]
    pub channel: Option<u32>,
    #[serde(default)]
    pub period_ns: Option<u64>,
    /// `normal` or `inversed`.
    #[serde(default)]
    pub polarity: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Fan {
    #[serde(default)]
    pub pwm: FanPwm,
    #[serde(default)]
    pub cooling_levels: Vec<u32>,
    #[serde(default)]
    pub min_level: Option<u32>,
    #[serde(default)]
    pub trips_c: Vec<f64>,
    #[serde(default, deserialize_with = "verified_map")]
    pub verified: std::collections::BTreeMap<String, Option<Verification>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct I2cDevice {
    #[serde(default)]
    pub id: Option<String>,
    pub part: String,
    pub bus: u32,
    pub address: I2cAddress,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfTestInfo {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub format_version: Option<u32>,
    #[serde(default)]
    pub checks: Vec<String>,
}

impl DeviceManifest {
    fn section<T: serde::de::DeserializeOwned>(&self, name: &str) -> Option<T> {
        serde_json::from_value(self.capabilities.get(name)?.clone()).ok()
    }

    /// `capabilities.leds`, when the manifest describes an LED ring.
    pub fn leds(&self) -> Option<LedRing> {
        self.section("leds")
    }

    /// `capabilities.fan`.
    pub fn fan(&self) -> Option<Fan> {
        self.section("fan")
    }

    /// `capabilities.i2c.devices`; devices that don't parse are left out.
    pub fn i2c_devices(&self) -> Vec<I2cDevice> {
        self.capabilities
            .get("i2c")
            .and_then(|i2c| i2c.get("devices"))
            .and_then(Value::as_array)
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|device| serde_json::from_value(device.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `capabilities.selftest`: the package has a self-test.
    pub fn selftest(&self) -> Option<SelfTestInfo> {
        self.section("selftest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_raze_package_reads() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../devices/raze/manifest.json"
        );
        let manifest: DeviceManifest =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let leds = manifest.leds().unwrap();
        assert_eq!(leds.wire_format.as_deref(), Some("grb24"));
        assert_eq!(leds.index.as_ref().map(|index| index.offset), Some(5));
        assert!(leds.verified["part"].is_some());
        assert!(leds.verified["count"].is_none());
        let fan = manifest.fan().unwrap();
        assert_eq!(fan.cooling_levels, vec![179, 212, 245, 255, 255]);
        assert_eq!(fan.pwm.period_ns, Some(41566));
        let devices = manifest.i2c_devices();
        assert_eq!(devices.len(), 4);
        assert_eq!(devices[0].address, I2cAddress(0x18));
        assert_eq!(
            manifest
                .selftest()
                .and_then(|selftest| selftest.format_version),
            Some(1)
        );
    }

    #[test]
    fn older_manifests_still_read() {
        let manifest: DeviceManifest = serde_json::from_str(
            r#"{ "contract": 1, "model": "raze", "capabilities": {
                 "leds": { "gpio": 13, "count": 16, "rgbw": true },
                 "i2c": { "devices": [ { "part": "BMI088", "bus": 4, "address": "0x18" },
                                       { "part": "odd", "bus": 1, "address": "0x99" } ] } } }"#,
        )
        .unwrap();
        let leds = manifest.leds().unwrap();
        assert_eq!(leds.count, Some(16));
        assert!(leds.index.is_none());
        assert!(manifest.fan().is_none());
        assert_eq!(
            manifest.i2c_devices(),
            vec![I2cDevice {
                id: None,
                part: "BMI088".into(),
                bus: 4,
                address: I2cAddress(0x18),
            }]
        );
        assert!(manifest.selftest().is_none());

        let listed: DeviceManifest =
            serde_json::from_str(r#"{ "contract": 1, "model": "x", "capabilities": ["fan"] }"#)
                .unwrap();
        assert!(listed.fan().is_none());
    }
}
