//! The steps a loader runs on a board in a Rockchip boot ROM: the maskrom
//! download, the wait for the usbplug to re-enumerate, the readiness check,
//! and the read-only queries that say what the board is.
//!
//! Which mode a device is in comes from the caller. The bus reports no
//! bcdUSB, and loader mode is told apart from maskrom by bit 0 of it, so a
//! caller that knows the device is already in loader mode skips
//! [`run_usbplug`] and calls [`ready`] on its own handle.

use std::time::Duration;

use atlas_usb::{OpenOptions, UsbBus, UsbIo};

use crate::UsbResult;
use crate::loader::Loader;
use crate::maskrom;
use crate::rockusb::{FlashInfo, Rockusb};

/// How often [`ready`] asks the usbplug whether it is up, and how many times.
const READY_TRIES: u32 = 10;
const READY_GAP: Duration = Duration::from_millis(500);

/// Runs `loader`'s 471 and 472 entries on the device at `location`, then
/// waits up to `reenumerate` for the board to come back at the same location
/// in loader mode. `file` is the loader's bytes, which `loader` was parsed
/// from. Returns the loader-mode handle.
pub async fn run_usbplug(
    bus: &dyn UsbBus,
    location: &str,
    loader: &Loader,
    file: &[u8],
    options: &OpenOptions,
    reenumerate: Duration,
) -> UsbResult<Box<dyn UsbIo>> {
    let mut io = bus.open(location, options).await?;
    maskrom::download_boot(io.as_mut(), loader, file).await?;
    drop(io);
    atlas_usb::await_device(bus, location, options, reenumerate).await
}

/// Asks the usbplug until it answers TEST UNIT READY, up to ten times half a
/// second apart. A stall or a protocol error is retried the same way; the
/// last one is returned.
pub async fn ready(io: &mut dyn UsbIo) -> UsbResult<()> {
    let mut device = Rockusb::new(io);
    let mut tries = 0;
    loop {
        match device.test_unit_ready().await {
            Ok(()) => return Ok(()),
            Err(error) if tries + 1 >= READY_TRIES => return Err(error),
            Err(_) => {
                tries += 1;
                tokio::time::sleep(READY_GAP).await;
            }
        }
    }
}

/// What a loader-mode board says about itself, read without changing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardReport {
    /// The chip number from READ_CHIP_INFO, reversed ASCII (see
    /// [`crate::loader::chip_number`]); `None` when the reply isn't one.
    pub chip: Option<String>,
    /// The raw READ_CHIP_INFO reply.
    pub chip_info: Vec<u8>,
    pub flash_id: Vec<u8>,
    pub flash: FlashInfo,
    pub capability: Vec<u8>,
    /// The storage the usbplug uses, as the index of its first bit set.
    pub storage: Option<u8>,
}

/// Reads the board report from a loader-mode handle. Each query is one
/// command; the first failure is returned as is.
pub async fn report(io: &mut dyn UsbIo) -> UsbResult<BoardReport> {
    let mut device = Rockusb::new(io);
    let chip_info = device.read_chip_info().await?;
    Ok(BoardReport {
        chip: crate::loader::chip_number(&chip_info),
        flash_id: device.read_flash_id().await?,
        flash: device.read_flash_info().await?,
        capability: device.read_capability().await?,
        storage: device.read_storage().await?,
        chip_info,
    })
}

/// Whether a loader's chip is the chip a board reports. `None` when either
/// number can't be read, which the caller decides how to treat.
pub fn chip_agrees(loader: &Loader, report: &BoardReport) -> Option<bool> {
    let board = report.chip.as_deref()?;
    Some(loader.chip_number()?.as_str() == board)
}
