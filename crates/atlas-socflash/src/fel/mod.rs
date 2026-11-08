//! Allwinner FEL: the boot ROM's USB download mode, flashing an SoC by
//! running an eGON SPL in SRAM through a thunk, then a legacy U-Boot image
//! from DRAM. Clean-room, MIT. Untested on a real A733 (the boards arrive
//! about 2026-11-18); checked against [`sim`] and the wire-format tests.
//!
//! Clean-room record. No sunxi-tools source was consulted, in any form, and
//! no U-Boot or Linux FEL code. The facts used are from these pages (the
//! linux-sunxi wiki, read from its web.archive.org snapshots,
//! `https://web.archive.org/web/2026id_/https://linux-sunxi.org/<Page>`):
//!
//! - FEL/Protocol (wiki revision oldid=26775, last edited 3 October 2025):
//!   the request, status and message block sizes and fields, the command
//!   numbers, the verify-device response, the memory notes.
//! - FEL (oldid=24514, 20 November 2021): how FEL is entered, the USB id
//!   1f3a:efe8, the version response's example.
//! - FEL/USBBoot (oldid=25215): the thunk's job (swap the FEL stacks with
//!   backup buffers around the SPL call, then return to the FEL loop), the
//!   legacy steps (SPL, then U-Boot to DRAM, then run), SRAM layout as data.
//! - BROM (oldid=23844): the eGON header's role at boot, the SPL load and
//!   stack addresses of the older SoCs (none for the A733).
//!
//! The wiki's example code and the references it gives to sunxi-tools were
//! not followed. The wording and the code here are our own.
//!
//! The stub. Running the SPL directly through FEL_RUN does not work for a
//! full SPL: the boot ROM's FEL stacks sit in SRAM the SPL needs, writing
//! over them stops the device answering, and the SPL must hand control back
//! to the ROM's FEL loop. So Atlas uses a thunk ([`thunk`], source
//! `fel/thunk.S`). It saves the registers, swaps each table buffer (its
//! stack side with the backup side), calls the SPL, swaps back and returns.
//! The SRAM it needs is data ([`layout`]). The thunk is assembled with
//! `clang --target=armv7a-none-eabi`, as briefed. The record says the A733
//! is AArch64 (`devices/radxa-cubie-a7a`), and the wiki's A64 note says that
//! SoC's FEL runs a 32-bit SPL, so the A733's FEL state is UNCONFIRMED: the
//! target may need to change once a board confirms it.
//!
//! UNCONFIRMED for the A733, all of it data: the SRAM layout (flash.sram in
//! the family is null), the thunk and SPL addresses, the DRAM address the
//! legacy U-Boot image goes to, the SoC id the version request reports, and
//! the thunk's ISA. Every run logs its transfers (target
//! `atlas_socflash::fel`), so the first real board shows where the device
//! answers differently.

pub mod client;
pub mod flash;
pub mod layout;
pub mod protocol;
pub mod sim;
pub mod thunk;

/// Allwinner's USB vendor id and the FEL product id of the A733 record.
pub const VENDOR_ID: u16 = atlas_usb_ids::ALLWINNER_VENDOR_ID;
pub const PRODUCT_ID: u16 = atlas_usb_ids::FEL_PRODUCT_ID;

/// The tracing target of every FEL transfer log.
pub const LOG_TARGET: &str = "atlas_socflash::fel";

/// One line per transfer step, in the same style as the K230 log.
pub(crate) fn trace(step: &str, request: &str, response: &[u8], expected: &str) {
    tracing::info!(
        target: LOG_TARGET,
        "{step}: request {request}; response {}; expected {expected}",
        crate::hex(response)
    );
}
