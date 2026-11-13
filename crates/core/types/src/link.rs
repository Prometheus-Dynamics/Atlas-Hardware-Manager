use serde::{Deserialize, Serialize};

use crate::{DeviceKey, Family, LinkId};

/// How Atlas reaches a device.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum LinkKind {
    /// USB gadget networking (ECM, RNDIS, NCM).
    UsbNetwork,
    /// Ethernet or any routed network, including a robot LAN.
    Ethernet,
    /// A USB serial port (CDC ACM or a USB-UART bridge).
    UsbSerial,
    /// A device in a USB boot ROM or bootloader mode.
    UsbBoot,
    /// A card reader or removable disk on this computer.
    LocalDisk,
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
            Self::LocalDisk => "local-disk",
            Self::Gateway { .. } => "gateway",
            Self::Simulated => "simulated",
        }
    }

    /// Whether two kinds are the same variant. A gateway matches any gateway,
    /// whichever device relays it.
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A physical or network path from this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: LinkId,
    pub kind: LinkKind,
    /// Human-readable description, for example `USB network (usb0)`.
    pub label: String,
}

/// Something found on a link that may be a device of one family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub link: LinkId,
    pub family: Family,
    /// Driver-specific address: an IP, a serial port path, a USB bus path.
    pub address: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateways_match_regardless_of_parent() {
        let a = LinkKind::Gateway {
            via: DeviceKey::from_static("sample-soc", "a"),
        };
        let b = LinkKind::Gateway {
            via: DeviceKey::from_static("sample-soc", "b"),
        };
        assert!(a.same_kind(&b));
        assert!(!a.same_kind(&LinkKind::Ethernet));
    }

    #[test]
    fn a_local_disk_is_its_own_kind_with_a_label() {
        assert_eq!(LinkKind::LocalDisk.label(), "local-disk");
        assert!(!LinkKind::LocalDisk.same_kind(&LinkKind::UsbBoot));
    }
}
