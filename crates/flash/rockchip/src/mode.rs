//! Maskrom or loader mode, told apart by the device's own USB descriptor.
//!
//! rkdeveloptool (RKScan.cpp) reads bit 0 of `bcdUSB`: clear in maskrom,
//! set once a loader runs. That is odd and not confirmed on the newest chips,
//! so [`Mode::of`] is only the default. A caller that knows better passes its
//! own [`Mode`] (the driver's `with_mode`, or `flow`'s callers that already
//! hold a loader-mode handle).

use atlas_usb::DeviceSummary;

/// Which mode a Rockchip boot ROM device is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The boot ROM: takes a loader's 471/472 stages.
    Maskrom,
    /// A usbplug or loader runs: takes rockusb commands.
    Loader,
}

impl Mode {
    /// The mode `bcd_usb` (the device's `bcdUSB`) says: bit 0 set is loader.
    pub const fn from_bcd_usb(bcd_usb: u16) -> Self {
        if bcd_usb & 1 == 0 {
            Self::Maskrom
        } else {
            Self::Loader
        }
    }

    /// The mode a bus summary reports, by its `bcdUSB`.
    pub const fn of(device: &DeviceSummary) -> Self {
        Self::from_bcd_usb(device.bcd_usb)
    }

    /// A short label for attributes and logs.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Maskrom => "maskrom",
            Self::Loader => "loader",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_bit_0_of_bcd_usb() {
        assert_eq!(Mode::from_bcd_usb(0x0200), Mode::Maskrom);
        assert_eq!(Mode::from_bcd_usb(0x0201), Mode::Loader);
        assert_eq!(Mode::from_bcd_usb(0), Mode::Maskrom);
    }

    #[test]
    fn a_summary_reports_the_mode_of_its_bcd_usb() {
        let mut device = DeviceSummary {
            location: "1-2".into(),
            vendor: 0x2207,
            product: 0x350b,
            serial: None,
            bcd_usb: 0x0200,
            bcd_device: 0x0100,
        };
        assert_eq!(Mode::of(&device), Mode::Maskrom);
        device.bcd_usb = 0x0201;
        assert_eq!(Mode::of(&device), Mode::Loader);
        assert_eq!(Mode::Loader.label(), "loader");
    }
}
