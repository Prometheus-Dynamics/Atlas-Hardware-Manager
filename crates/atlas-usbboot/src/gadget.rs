//! A Pi that already finished USB boot with the mass-storage-gadget files:
//! its eMMC is exposed as a USB disk and it waits to be written. Atlas finds
//! these too, so a flash that stopped after USB boot (or a board booted by
//! another tool) can be written without power-cycling it.

use lemnos_usb::UsbDeviceInfo as DeviceInfo;
use serde::Serialize;

use crate::{BROADCOM_VENDOR_ID, UsbBootError};

/// The USB product id of Raspberry Pi's mass-storage gadget
/// ("Raspberry Pi multi-function USB device").
pub const STORAGE_GADGET_PRODUCT_ID: u16 = 0x0104;
const MASS_STORAGE_CLASS: u8 = 0x08;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StorageGadget {
    /// Same form as [`BootDevice::location`](crate::BootDevice), so the
    /// board keeps its location across USB boot.
    pub location: String,
    /// Linux sysfs form (`1-4.2`), to match the disk on this port.
    pub port: String,
    /// The gadget's serial; its last 8 hex digits are the boot ROM serial.
    pub serial: Option<String>,
}

impl StorageGadget {
    /// The boot ROM serial, so the board keeps its key across USB boot.
    pub fn board_serial(&self) -> Option<String> {
        let serial = self.serial.as_deref()?.trim();
        let tail = serial.get(serial.len().checked_sub(8)?..)?;
        tail.chars()
            .all(|c| c.is_ascii_hexdigit())
            .then(|| tail.to_ascii_lowercase())
    }
}

fn is_storage_gadget(info: &DeviceInfo) -> bool {
    info.vendor_id() == BROADCOM_VENDOR_ID
        && info.product_id() == STORAGE_GADGET_PRODUCT_ID
        && info
            .interfaces()
            .iter()
            .any(|interface| interface.class == MASS_STORAGE_CLASS)
}

/// Every Pi currently exposing its eMMC as a USB disk.
pub async fn list_storage_gadgets() -> Result<Vec<StorageGadget>, UsbBootError> {
    Ok(crate::usb::list_all()
        .await?
        .into_iter()
        .filter(is_storage_gadget)
        .map(|info| {
            // Lemnos's location is the sysfs form already (`1-4.2`).
            let location = info.location().to_string();
            StorageGadget {
                port: location.clone(),
                location,
                serial: info.serial_number().map(str::to_string),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gadget(serial: Option<&str>) -> StorageGadget {
        StorageGadget {
            location: "1-4".into(),
            port: "1-4".into(),
            serial: serial.map(str::to_string),
        }
    }

    #[test]
    fn the_board_serial_is_the_tail_of_the_gadget_serial() {
        // Seen on a real Raze: boot ROM e5226d57, gadget a317bcbee5226d57.
        assert_eq!(
            gadget(Some("a317bcbee5226d57")).board_serial().as_deref(),
            Some("e5226d57")
        );
        assert_eq!(gadget(Some("short")).board_serial(), None);
        assert_eq!(gadget(Some("zzzzzzzzzz")).board_serial(), None);
        assert_eq!(gadget(None).board_serial(), None);
    }
}
