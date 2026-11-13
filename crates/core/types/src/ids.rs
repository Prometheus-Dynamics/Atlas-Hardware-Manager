use std::fmt;

use serde::{Deserialize, Serialize};

/// A device family, such as `sample-soc`. One driver owns one family.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Family(String);

impl Family {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A hardware serial number as reported by the device.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Serial(String);

impl Serial {
    pub fn new(serial: impl Into<String>) -> Self {
        Self(serial.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The stable key for a device: its family plus its serial.
///
/// Never an address or a port, which change as a device moves between links.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeviceKey {
    pub family: Family,
    pub serial: Serial,
}

impl DeviceKey {
    pub fn new(family: impl Into<String>, serial: impl Into<String>) -> Self {
        Self {
            family: Family::new(family),
            serial: Serial::new(serial),
        }
    }

    /// Parses the `family:serial` form produced by [`fmt::Display`].
    pub fn parse(text: &str) -> Option<Self> {
        let (family, serial) = text.split_once(':')?;
        let (family, serial) = (family.trim(), serial.trim());
        if family.is_empty() || serial.is_empty() {
            return None;
        }
        Some(Self::new(family, serial))
    }
}

impl fmt::Display for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.family, self.serial)
    }
}

/// Identifies one link for the lifetime of a scan, for example `usbnet:enp0s20u1`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkId(String);

impl LinkId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_round_trips_through_display() {
        let key = DeviceKey::new("sample-soc", "1000abcd");
        assert_eq!(key.to_string(), "sample-soc:1000abcd");
        assert_eq!(DeviceKey::parse("sample-soc:1000abcd"), Some(key));
    }

    #[test]
    fn device_key_parse_rejects_missing_parts() {
        assert_eq!(DeviceKey::parse("sample-soc"), None);
        assert_eq!(DeviceKey::parse(":abc"), None);
        assert_eq!(DeviceKey::parse("sample-soc: "), None);
    }
}
