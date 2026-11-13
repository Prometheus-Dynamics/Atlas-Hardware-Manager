//! The Kendryte K230 USB download mode, in Rust, over `atlas-usb`.
//! No vendor tool runs.
//!
//! Two stages answer on one USB id, which the caller passes in:
//!
//! - [`bootrom`]: the boot ROM. It takes a RAM image into memory at an
//!   address the caller gives, and starts it. The image is the burner (Canaan's
//!   "kburn", a U-Boot) or the flasher image. Starting is the same either way.
//! - [`burner`]: the read-only queries of a running burner: its stage, the
//!   medium it would burn, and reads from it.
//! - [`write`]: the burner's write path (erase, write, verify, reboot). It is
//!   kept apart on purpose: nothing a boot needs calls it.
//!
//! Follows Canaan's k230_flash (33a75f0, BSD-2-Clause). The boot ROM and the
//! burner's source are not public, so the byte layouts come from that tool.
//! Nothing here is confirmed on a board: every transfer is logged at target
//! `atlas_k230` with the answer the C source predicts, so the first real
//! plug-in shows where the two differ.

pub mod bootrom;
pub mod burner;
pub mod packet;
pub mod write;

/// The tracing target of every transfer log.
pub const LOG_TARGET: &str = "atlas_k230";

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Hex of `bytes`, or the first 64 bytes and the total length.
pub(crate) fn show(bytes: &[u8]) -> String {
    if bytes.len() > 64 {
        format!("{}.. ({} bytes)", hex(&bytes[..64]), bytes.len())
    } else {
        hex(bytes)
    }
}

/// One line per transfer of a step no board has confirmed yet: what was
/// sent, what came back, and what the C source predicts.
pub(crate) fn trace(step: &str, request: &str, response: &[u8], expected: &str) {
    tracing::info!(
        target: LOG_TARGET,
        "{step}: request {request}; response {}; unconfirmed, expected {expected}",
        show(response)
    );
}
