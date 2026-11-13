//! Allwinner FEL: the boot ROM's USB download mode. Atlas loads a board from
//! its boot ROM with this crate, in Rust, with no vendor tool.
//!
//! The flow: verify the device is in FEL, write the SPL into SRAM through a
//! thunk (see [`thunk`]), read it back, run the thunk, check that FEL answers
//! again, then write an image to DRAM, read it back and run it.
//!
//! Built on the traits of `atlas-usb` ([`UsbIo`]) and nothing else of Atlas.
//! Files, USB ids and the SRAM layout come in from the caller as data.
//!
//! Clean-room, MIT. The code was written from public protocol notes (the
//! linux-sunxi wiki's FEL, FEL/Protocol, FEL/USBBoot and BROM pages, read from
//! web archive snapshots). No GPL source (sunxi-tools) and no vendor or U-Boot
//! FEL code was consulted.
//!
//! UNCONFIRMED on the A733 and untested on a board. Carried as data, never as
//! constants: the SoC's SRAM layout, the thunk's instruction set (the thunk
//! here is ARMv7; the A733 is AArch64 by its record, see [`layout::ThunkIsa`]),
//! the DRAM address the image loads to, and the SoC id the device reports.
//! Every transfer is logged at `info` under [`LOG_TARGET`].

pub mod client;
pub mod flash;
pub mod layout;
pub mod protocol;
pub mod sim;
pub mod thunk;

pub use atlas_usb::{UsbError, UsbIo};

/// The tracing target of every FEL transfer log.
pub const LOG_TARGET: &str = "atlas_fel";

/// One line per command step: what was sent and what came back.
pub(crate) fn trace(step: &str, request: &str, response: &[u8], expected: &str) {
    tracing::info!(
        target: LOG_TARGET,
        "{step}: request {request}; response {}; expected {expected}",
        hex(response)
    );
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
