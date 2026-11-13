//! The Broadcom SoCs a Pi boot ROM can be: what each one expects as its second
//! stage, and the folder its boot files live in. Which USB id is which SoC is
//! catalog data the caller passes in; this crate names no USB ids.

/// A boot ROM's SoC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chip {
    Bcm2835,
    Bcm2837,
    Bcm2711,
    Bcm2712,
}

impl Chip {
    /// The folder for this chip inside a boot files tree.
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

    /// The SoC's name, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bcm2835 => "BCM2835",
            Self::Bcm2837 => "BCM2837",
            Self::Bcm2711 => "BCM2711",
            Self::Bcm2712 => "BCM2712",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cm4_and_cm5_use_their_own_second_stage_and_folder() {
        assert_eq!(Chip::Bcm2711.second_stage_file(), "bootcode4.bin");
        assert_eq!(Chip::Bcm2712.second_stage_file(), "bootcode5.bin");
        assert_eq!(Chip::Bcm2712.folder(), "2712");
        assert_eq!(Chip::Bcm2837.folder(), "2710");
    }
}
