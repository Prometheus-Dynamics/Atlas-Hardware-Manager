//! Rockchip maskrom and loader mode (rockusb), as rkdeveloptool speaks it,
//! with no vendor tool at run time: [`maskrom`] runs a loader file's DDR
//! init and usbplug, [`rockusb`] is the usbplug's command set, [`write`]
//! puts images, idblocks and UBI on the storage.
//!
//! Written from rkdeveloptool, U-Boot's rockusb and rkcommon.c; tested
//! against traces built from those sources. Untested on a board.

pub mod crc;
pub mod loader;
pub mod maskrom;
pub mod rc4;
pub mod rockusb;
pub mod write;

use crate::transport::{UsbError, location};

pub const VENDOR_ID: u16 = 0x2207;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// The boot ROM: takes a loader's 471/472 stages.
    Maskrom,
    /// A usbplug or loader runs: takes rockusb commands.
    Loader,
}

impl Mode {
    /// rkdeveloptool (RKScan.cpp) tells them apart by bit 0 of bcdUSB:
    /// clear in maskrom, set in loader mode. Odd, and unconfirmed on the
    /// newest chips; it's what Rockchip's own tool relies on.
    pub fn from_bcd_usb(bcd_usb: u16) -> Self {
        if bcd_usb & 1 == 0 {
            Self::Maskrom
        } else {
            Self::Loader
        }
    }
}

/// A Rockchip SoC on USB in maskrom or loader mode.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Device {
    pub location: String,
    pub product_id: u16,
    pub mode: Mode,
    pub serial: Option<String>,
}

/// Every Rockchip device in a download mode. Rockchip's mass-storage PIDs
/// (`2207:0000`, `2207:0010`) are a running board's, not a download mode.
pub async fn list() -> Result<Vec<(Device, nusb::DeviceInfo)>, UsbError> {
    let devices = nusb::list_devices()
        .await
        .map_err(|error| UsbError::Other(error.to_string()))?;
    Ok(devices
        .filter(|info| info.vendor_id() == VENDOR_ID && info.product_id() >> 8 != 0)
        .map(|info| {
            let device = Device {
                location: location(&info),
                product_id: info.product_id(),
                mode: Mode::from_bcd_usb(info.usb_version()),
                serial: info
                    .serial_number()
                    .map(str::trim)
                    .filter(|serial| !serial.is_empty())
                    .map(str::to_string),
            };
            (device, info)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_bit_0_of_bcd_usb() {
        assert_eq!(Mode::from_bcd_usb(0x0200), Mode::Maskrom);
        assert_eq!(Mode::from_bcd_usb(0x0201), Mode::Loader);
    }
}
