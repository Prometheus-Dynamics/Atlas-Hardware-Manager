//! The steps a loader runs on a board in a Rockchip boot ROM: the maskrom
//! download, the wait for the usbplug to re-enumerate, the readiness check,
//! and the read-only queries that say what the board is.
//!
//! Which mode a device is in comes from [`crate::Mode`], read from the
//! device's `bcdUSB`, or from the caller when it knows better. A caller that
//! knows the device is already in loader mode skips [`run_usbplug`] and calls
//! [`ready`] on its own handle.

use std::time::Duration;

use atlas_usb::{
    Expect, OpenOptions, Opened, Outcome, PortPath, UsbBus, UsbId, UsbIo, observe_stop,
    open_expected,
};

use crate::UsbResult;
use crate::loader::Loader;
use crate::maskrom;
use crate::rockusb::{FlashInfo, Rockusb};

/// How often [`ready`] asks the usbplug whether it is up, and how many times.
const READY_TRIES: u32 = 10;
const READY_GAP: Duration = Duration::from_millis(500);

/// Runs `loader`'s 471 and 472 entries on the device on `port`, then waits up to
/// `reenumerate` for the board to come back on the same port in loader mode.
/// `file` is the loader's bytes, which `loader` was parsed from.
///
/// The device must hold `expected` on `port` before the download starts; a
/// different identity there is refused and nothing is written. The loader-mode
/// device that comes back has an identity this crate does not know in advance,
/// so only its port selects it.
pub async fn run_usbplug(
    bus: &dyn UsbBus,
    port: &PortPath,
    expected: UsbId,
    loader: &Loader,
    file: &[u8],
    options: &OpenOptions,
    reenumerate: Duration,
) -> Outcome<Opened> {
    let mut opened = match open_expected(
        bus,
        port,
        Expect::Exactly(expected),
        options,
        Duration::ZERO,
    )
    .await
    {
        Outcome::Done(opened) => opened,
        stopped => return stopped,
    };
    if let Err(error) = maskrom::download_boot(opened.io.as_mut(), loader, file).await {
        return observe_stop(
            bus,
            port,
            Expect::Exactly(expected),
            "the loader's download",
            options.timeouts.bulk,
            true,
            error,
        )
        .await
        .typed();
    }
    drop(opened);
    open_expected(bus, port, Expect::AnyIdentity, options, reenumerate).await
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
