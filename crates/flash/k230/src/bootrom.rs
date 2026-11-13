//! The K230 boot ROM's USB download mode: EP0 vendor requests read the CPU
//! info and set the load address, a RAM image goes over bulk out in pages,
//! then a request starts it. The board resets as the image starts, so the
//! device comes back at the same port as whatever image was started.
//!
//! Checked against k230_flash (33a75f0): burner_brom.cpp, probe_device.cpp,
//! kburn_k230.h. Nothing here was run on a board.

use std::ops::RangeInclusive;

use atlas_usb::{UsbError, UsbIo};

use crate::trace;

/// kburn_k230.h:27-31: EP0_GET_CPU_INFO 0, EP0_SET_DATA_ADDRESS 1,
/// EP0_PROG_START 4.
pub const GET_CPU_INFO: u8 = 0;
pub const SET_DATA_ADDRESS: u8 = 1;
pub const PROG_START: u8 = 4;
/// probe_device.cpp:23: wLength 32.
pub const CPU_INFO_LEN: u16 = 32;
/// The boot ROM's bulk page: burner_brom.cpp `K230_SRAM_PAGE_SIZE` (1000).
pub const PAGE: usize = 1000;
/// The range k230_flash accepts for a load address (k230_flash.cpp:109). The
/// boot ROM's own limit is UNCONFIRMED. The caller always passes the address;
/// nothing here picks one.
pub const LOAD_ADDRESS_RANGE: RangeInclusive<u32> = 0x8030_0000..=0x8040_0000;

/// Which stage answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The boot ROM ("K230").
    BootRom,
    /// The burner U-Boot ("Uboot Stage for K230").
    Burner,
    /// Something else answered, with its text.
    Unknown(String),
}

/// probe_device.cpp:9-61: request 0, wValue 0, 32 bytes. "Uboot Stage for
/// K230" is the burner, "K230" the boot ROM.
pub async fn stage(io: &mut dyn UsbIo) -> Result<Stage, UsbError> {
    let reply = io.control_in(GET_CPU_INFO, 0, 0, CPU_INFO_LEN).await?;
    trace(
        "bootrom cpu info",
        "IN vendor req 0 wValue 0 wIndex 0 len 32",
        &reply,
        "\"K230\" (boot ROM) or \"Uboot Stage for K230\" (burner), NUL padded",
    );
    let text = String::from_utf8_lossy(&reply)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    Ok(match text.as_str() {
        "Uboot Stage for K230" => Stage::Burner,
        "K230" => Stage::BootRom,
        _ => Stage::Unknown(text),
    })
}

/// Refuses an address outside [`LOAD_ADDRESS_RANGE`] before any transfer.
pub fn check_address(address: u32) -> Result<(), UsbError> {
    if LOAD_ADDRESS_RANGE.contains(&address) {
        Ok(())
    } else {
        Err(UsbError::Other(format!(
            "load address {address:#010x} is outside {:#010x}..={:#010x}",
            LOAD_ADDRESS_RANGE.start(),
            LOAD_ADDRESS_RANGE.end()
        )))
    }
}

/// burner_brom.cpp:73-74: wValue the high 16 bits, wIndex the low 16.
fn split(address: u32) -> (u16, u16) {
    ((address >> 16) as u16, (address & 0xffff) as u16)
}

/// Loads `image` at `address` and starts it at that address.
///
/// The image is whatever the caller supplies: the burner, or the flasher
/// image. Starting is the same request for both. A disconnect at the start
/// request is the boot ROM resetting into the image, so it is success. A
/// disconnect anywhere before that is an error: the image did not load, and
/// no start was sent.
pub async fn load_and_start(
    io: &mut dyn UsbIo,
    image: &[u8],
    address: u32,
) -> Result<(), UsbError> {
    check_address(address)?;
    if image.is_empty() {
        return Err(UsbError::Other("there is no image to load".into()));
    }
    let (high, low) = split(address);
    io.control_out(SET_DATA_ADDRESS, high, low, &[]).await?;
    trace(
        "bootrom set data address",
        &format!("OUT vendor req 1 wValue {high:#06x} wIndex {low:#06x} len 0"),
        &[],
        "success, no data stage",
    );
    // burner_brom.cpp:119-156: whole pages, the last one short.
    for page in image.chunks(PAGE) {
        io.bulk_out(page).await?;
    }
    trace(
        "bootrom image upload",
        &format!(
            "bulk OUT {} bytes in {} pages of up to {PAGE}",
            image.len(),
            image.len().div_ceil(PAGE)
        ),
        &image[..image.len().min(64)],
        "every page accepted in full",
    );
    let started = io.control_out(PROG_START, high, low, &[]).await;
    trace(
        "bootrom program start",
        &format!("OUT vendor req 4 wValue {high:#06x} wIndex {low:#06x} len 0"),
        &[],
        "success, then the board re-enumerates",
    );
    match started {
        // The C tool treats every failure as one (burner_brom.cpp:30-35). A
        // disconnect here is the ROM jumping into the image.
        Err(UsbError::Disconnected) => Ok(()),
        other => other,
    }
}
