use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A device family, such as `helios` or `stm32`. One driver owns one family.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Family(pub String);

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
pub struct Serial(pub String);

impl Serial {
    pub fn new(serial: impl Into<String>) -> Self {
        Self(serial.into())
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The stable key for a device: family plus serial.
///
/// Never an IP address or a USB port, which change as devices move between links.
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

    /// Parses the `family:serial` form produced by [`Display`](fmt::Display).
    pub fn parse(text: &str) -> Option<Self> {
        let (family, serial) = text.split_once(':')?;
        let family = family.trim();
        let serial = serial.trim();
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
pub struct LinkId(pub String);

impl fmt::Display for LinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How Atlas reaches a device.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum LinkKind {
    /// USB gadget networking (ECM, RNDIS, NCM).
    UsbNetwork,
    /// Ethernet or any routed network, including the robot LAN.
    Ethernet,
    /// A USB serial port (CDC ACM or a USB-UART bridge).
    UsbSerial,
    /// A device in a USB bootloader or boot-ROM mode.
    UsbBoot,
    /// Reached through another device that relays to it.
    Gateway { via: DeviceKey },
    /// Simulated devices for tests and UI work.
    Simulated,
}

impl LinkKind {
    /// A short label for tables and logs.
    pub fn label(&self) -> &'static str {
        match self {
            Self::UsbNetwork => "usb-net",
            Self::Ethernet => "ethernet",
            Self::UsbSerial => "usb-serial",
            Self::UsbBoot => "usb-boot",
            Self::Gateway { .. } => "gateway",
            Self::Simulated => "simulated",
        }
    }

    /// Whether two kinds are the same variant, ignoring gateway details.
    pub fn same_variant(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A physical or network path from this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: LinkId,
    pub kind: LinkKind,
    /// Human-readable description, for example `USB network (enp0s20u1)`.
    pub label: String,
}

/// Something a driver found on a link that may be one of its devices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub link: LinkId,
    pub family: Family,
    /// Driver-specific address: an IP, a serial port path, a USB bus path.
    pub address: String,
}

/// Whether a device is running normally or waiting in a recovery mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceMode {
    #[default]
    Normal,
    /// Bootloader, boot ROM, or another state that only accepts recovery.
    Recovery,
}

/// Who a device is, as reported by the device itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub key: DeviceKey,
    /// Hardware model or board name.
    pub model: String,
    pub mode: DeviceMode,
    /// Named versions, for example `os`, `firmware`, `bootloader`, `api`.
    pub versions: BTreeMap<String, String>,
    /// The name stored on the device, if it has one.
    pub name: Option<String>,
    /// Where the device was reached during this scan.
    pub link: LinkId,
    pub address: String,
    /// Driver-specific facts shown in the device panel, for example `os`,
    /// `revision`, `hostname`, `manage_url`, `mac.usb0`.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

impl Identity {
    /// The version shown in the inventory: `os`, then `firmware`, then `bootloader`.
    pub fn primary_version(&self) -> Option<&str> {
        ["os", "firmware", "bootloader"]
            .iter()
            .find_map(|name| self.versions.get(*name))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_round_trips_through_display() {
        let key = DeviceKey::new("helios", "1000abcd");
        assert_eq!(key.to_string(), "helios:1000abcd");
        assert_eq!(DeviceKey::parse("helios:1000abcd"), Some(key));
    }

    #[test]
    fn device_key_parse_rejects_missing_parts() {
        assert_eq!(DeviceKey::parse("helios"), None);
        assert_eq!(DeviceKey::parse(":abc"), None);
        assert_eq!(DeviceKey::parse("helios: "), None);
    }

    #[test]
    fn gateway_links_match_variant_regardless_of_parent() {
        let a = LinkKind::Gateway {
            via: DeviceKey::new("helios", "a"),
        };
        let b = LinkKind::Gateway {
            via: DeviceKey::new("helios", "b"),
        };
        assert!(a.same_variant(&b));
        assert!(!a.same_variant(&LinkKind::Ethernet));
    }
}
