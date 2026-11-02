//! Kendryte K230/K230D USB burning, as Canaan's k230_burning_tool speaks it
//! (canaan-burn handshake.k230.c, k230_burn.c; the protocol notes of the
//! k230_flash Python port), with no vendor tool at run time:
//!
//! - [`bootrom`]: the boot ROM's download mode takes a loader (the burner
//!   U-Boot, "kburn") into memory and starts it;
//! - [`kburn`]: the burner's 60-byte command packets: pick the medium,
//!   erase, write, verify, reboot.
//!
//! Both stages enumerate as 29f1:0230; the CPU-info request tells them
//! apart. The boot ROM and the burner's own source aren't public, so this
//! follows the host tools. Tested against traces built from them;
//! untested on a board.

pub mod bootrom;
pub mod firmware;
pub mod kburn;

use crate::transport::{DeviceInfo, UsbError, list_all, location};

pub const VENDOR_ID: u16 = 0x29f1;
pub const PRODUCT_ID: u16 = 0x0230;

/// A K230 in USB download mode (either stage).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Device {
    pub location: String,
    pub serial: Option<String>,
}

pub async fn list() -> Result<Vec<(Device, DeviceInfo)>, UsbError> {
    Ok(list_all()
        .await?
        .into_iter()
        .filter(|info| info.vendor_id() == VENDOR_ID && info.product_id() == PRODUCT_ID)
        .map(|info| {
            let device = Device {
                location: location(&info),
                serial: info.serial_number().map(str::to_string),
            };
            (device, info)
        })
        .collect())
}
