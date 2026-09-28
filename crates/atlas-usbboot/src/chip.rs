use serde::{Deserialize, Serialize};

/// Broadcom's USB vendor id, used by every Pi boot ROM.
pub const BROADCOM_VENDOR_ID: u16 = 0x0a5c;

/// The Pi SoC behind a USB boot device, from its product id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Chip {
    /// BCM2835 (Pi 1, Zero, CM1).
    Bcm2835,
    /// BCM2836/BCM2837 (Pi 2, 3, CM3).
    Bcm2837,
    /// BCM2711 (Pi 4, CM4).
    Bcm2711,
    /// BCM2712 (Pi 5, CM5).
    Bcm2712,
}

impl Chip {
    pub fn from_product_id(product_id: u16) -> Option<Self> {
        match product_id {
            0x2763 => Some(Self::Bcm2835),
            0x2764 => Some(Self::Bcm2837),
            0x2711 => Some(Self::Bcm2711),
            0x2712 => Some(Self::Bcm2712),
            _ => None,
        }
    }

    pub fn product_id(self) -> u16 {
        match self {
            Self::Bcm2835 => 0x2763,
            Self::Bcm2837 => 0x2764,
            Self::Bcm2711 => 0x2711,
            Self::Bcm2712 => 0x2712,
        }
    }

    /// The folder name for this chip inside `bootfiles.bin` and boot dirs.
    pub fn folder(self) -> &'static str {
        match self {
            Self::Bcm2835 | Self::Bcm2837 => "2710",
            Self::Bcm2711 => "2711",
            Self::Bcm2712 => "2712",
        }
    }

    /// The second-stage bootloader the boot ROM expects.
    pub fn second_stage_file(self) -> &'static str {
        match self {
            Self::Bcm2835 | Self::Bcm2837 => "bootcode.bin",
            Self::Bcm2711 => "bootcode4.bin",
            Self::Bcm2712 => "bootcode5.bin",
        }
    }

    /// A human-readable name, for example `BCM2711 (Pi 4 / CM4)`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bcm2835 => "BCM2835 (Pi 1 / Zero / CM1)",
            Self::Bcm2837 => "BCM2837 (Pi 3 / CM3)",
            Self::Bcm2711 => "BCM2711 (Pi 4 / CM4)",
            Self::Bcm2712 => "BCM2712 (Pi 5 / CM5)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_ids_round_trip() {
        for chip in [Chip::Bcm2835, Chip::Bcm2837, Chip::Bcm2711, Chip::Bcm2712] {
            assert_eq!(Chip::from_product_id(chip.product_id()), Some(chip));
        }
        assert_eq!(Chip::from_product_id(0x1234), None);
    }

    #[test]
    fn cm4_and_cm5_use_their_own_second_stage() {
        assert_eq!(Chip::Bcm2711.second_stage_file(), "bootcode4.bin");
        assert_eq!(Chip::Bcm2712.second_stage_file(), "bootcode5.bin");
        assert_eq!(Chip::Bcm2712.folder(), "2712");
    }
}
