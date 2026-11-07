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
//! follows the host tool, Canaan's k230_flash (33a75f0, BSD-2-Clause:
//! kburn.h, kburn_k230.h, burner_brom.cpp, burner_uboot.cpp,
//! probe_device.cpp). Tested against byte vectors derived from it
//! (tests/k230_vectors.rs); untested on a board, so every transfer logs
//! its request and response (target `atlas_socflash::k230`).

pub mod bootrom;
pub mod firmware;
pub mod kburn;

use crate::transport::{UsbError, location};

/// k230_flash kburn.h:205-206 (VID 0x29f1 / PID 0x0230); verified against
/// the C source, not against a board.
pub const VENDOR_ID: u16 = 0x29f1;
pub const PRODUCT_ID: u16 = 0x0230;

/// The tracing target of every K230 transfer log.
pub const LOG_TARGET: &str = "atlas_socflash::k230";

/// Hex of `bytes`, or the first 64 bytes and the total length.
pub(crate) fn show(bytes: &[u8]) -> String {
    if bytes.len() > 64 {
        format!("{}.. ({} bytes)", crate::hex(&bytes[..64]), bytes.len())
    } else {
        crate::hex(bytes)
    }
}

/// One line per transfer of a step nothing has confirmed on a board yet:
/// what was sent, what came back, and the answer the C source predicts, so
/// the first real plug-in shows where the two differ.
pub(crate) fn trace(step: &str, request: &str, response: &[u8], expected: &str) {
    tracing::info!(
        target: "atlas_socflash::k230",
        "{step}: request {request}; response {}; unconfirmed, expected {expected}",
        show(response)
    );
}

/// A K230 in USB download mode (either stage).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Device {
    pub location: String,
    pub serial: Option<String>,
}

pub async fn list() -> Result<Vec<(Device, nusb::DeviceInfo)>, UsbError> {
    let devices = nusb::list_devices()
        .await
        .map_err(|error| UsbError::Other(error.to_string()))?;
    Ok(devices
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
