//! The boot-ROM steps against the simulated board on the shared USB bus:
//! the maskrom download and the usbplug's re-enumeration, a stalled transfer,
//! a timed-out one, a stage with a bad CRC, a disconnect, the readiness
//! retries, and the rockusb report. Time is paused, so the timeouts run at
//! once.

use std::time::Duration;

use atlas_rockchip::Mode;
use atlas_rockchip::crc::LOADER_POLYNOMIALS;
use atlas_rockchip::flow::{chip_agrees, ready, report, run_usbplug};
use atlas_rockchip::loader::{EntryKind, Loader, build_loader};
use atlas_rockchip::maskrom::{self, STAGE_DDR, STAGE_USBPLUG};
use atlas_rockchip::sim::{Fault, SimRockchip};
use atlas_usb::{OpenOptions, UsbBus, UsbError, UsbId};
use tokio::time::Instant;

const ID: UsbId = UsbId {
    vendor: 0x2207,
    product: 0x350b,
};
const LOCATION: &str = "1-2";
const REENUMERATE: Duration = Duration::from_secs(10);

/// A loader whose DDR stage is 5000 bytes and whose usbplug is 4094 bytes:
/// the usbplug takes the lone trailing pad byte, so both framings are on
/// the wire. The header says chip `chip` (reversed ASCII).
fn loader_file(chip: &[u8; 4]) -> Vec<u8> {
    let ddr = vec![0x11u8; 5000];
    let usbplug = vec![0x22u8; 4094];
    let head = [3u8; 16];
    build_loader(
        chip,
        &[
            (EntryKind::Ddr, "ddr", &ddr, 0),
            (EntryKind::Usbplug, "usbplug", &usbplug, 0),
            (EntryKind::Loader, "FlashHead", &head, 0),
        ],
        LOADER_POLYNOMIALS[0],
    )
}

fn maskrom_board() -> SimRockchip {
    SimRockchip::maskrom(LOCATION, ID, "3588", 1000)
}

#[tokio::test(start_paused = true)]
async fn maskrom_download_reenumerates_in_loader_mode_and_reports_the_board() {
    let sim = maskrom_board();
    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    let mut io = run_usbplug(
        &sim,
        LOCATION,
        &loader,
        &file,
        &OpenOptions::default(),
        REENUMERATE,
    )
    .await
    .unwrap();

    // DDR init first, then the usbplug; the board is in loader mode after.
    assert_eq!(sim.stages(), vec![STAGE_DDR, STAGE_USBPLUG]);
    assert!(sim.in_loader_mode());

    ready(io.as_mut()).await.unwrap();
    let board = report(io.as_mut()).await.unwrap();
    assert_eq!(board.chip.as_deref(), Some("3588"));
    assert_eq!(chip_agrees(&loader, &board), Some(true));
    assert_eq!(board.flash.sectors, 1000);
    assert_eq!(board.flash.bytes(), 512_000);
    assert_eq!(board.flash_id, vec![0; 5]);
    assert_eq!(board.capability, vec![0; 8]);
    assert_eq!(board.storage, Some(1));
}

#[tokio::test(start_paused = true)]
async fn the_maskrom_handle_dies_when_the_usbplug_comes_up() {
    let sim = maskrom_board();
    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    let mut old = sim.open(LOCATION, &OpenOptions::default()).await.unwrap();
    maskrom::download_boot(old.as_mut(), &loader, &file)
        .await
        .unwrap();

    // The handle from before the reset is dead; a new one at the same
    // location is the loader.
    assert_eq!(old.bulk_out(&[0; 31]).await, Err(UsbError::Disconnected));
    let mut fresh = sim.open(LOCATION, &OpenOptions::default()).await.unwrap();
    ready(fresh.as_mut()).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stalled_stage_fails_and_leaves_the_board_in_maskrom_for_a_retry() {
    let sim = maskrom_board();
    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    let options = OpenOptions::default();

    sim.inject(Fault::Stall);
    let error = run_usbplug(&sim, LOCATION, &loader, &file, &options, REENUMERATE)
        .await
        .err();
    assert_eq!(error, Some(UsbError::Stall));
    assert!(sim.stages().is_empty());
    assert!(!sim.in_loader_mode());

    // The same board takes the whole download again.
    run_usbplug(&sim, LOCATION, &loader, &file, &options, REENUMERATE)
        .await
        .unwrap();
    assert!(sim.in_loader_mode());
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_stage_waits_for_its_timeout_then_fails() {
    let sim = maskrom_board();
    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    sim.inject(Fault::Timeout);

    let started = Instant::now();
    let error = run_usbplug(
        &sim,
        LOCATION,
        &loader,
        &file,
        &OpenOptions::default(),
        REENUMERATE,
    )
    .await
    .err();
    assert!(matches!(error, Some(UsbError::Timeout(_))), "got {error:?}");
    // The default control timeout is five seconds.
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert!(sim.stages().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_stage_that_arrives_with_a_bad_crc_is_refused() {
    let sim = maskrom_board();
    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    // One byte flipped on its way to the board: the stage's CRC no longer
    // matches when the stage closes.
    sim.inject(Fault::CorruptChunk);

    let error = run_usbplug(
        &sim,
        LOCATION,
        &loader,
        &file,
        &OpenOptions::default(),
        REENUMERATE,
    )
    .await
    .err();
    match error {
        Some(UsbError::Protocol(message)) => {
            assert!(message.contains("CRC"), "{message}");
            assert!(message.contains("0x0471"), "{message}");
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
    assert!(sim.stages().is_empty());
    assert!(!sim.in_loader_mode());
}

#[tokio::test(start_paused = true)]
async fn a_board_that_drops_off_mid_download_is_disconnected_and_gone() {
    let sim = maskrom_board();
    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    sim.inject(Fault::Disconnect);

    let error = run_usbplug(
        &sim,
        LOCATION,
        &loader,
        &file,
        &OpenOptions::default(),
        REENUMERATE,
    )
    .await
    .err();
    assert_eq!(error, Some(UsbError::Disconnected));
    // Nothing is at the location any more, so a retry says so at once.
    let again = sim.open(LOCATION, &OpenOptions::default()).await.err();
    assert_eq!(again, Some(UsbError::NotFound(LOCATION.into())));
}

#[tokio::test(start_paused = true)]
async fn a_device_already_in_loader_mode_is_read_without_a_download() {
    let sim = SimRockchip::loader(LOCATION, ID, "3588", 1000);
    let mut io = sim.open(LOCATION, &OpenOptions::default()).await.unwrap();
    ready(io.as_mut()).await.unwrap();
    let board = report(io.as_mut()).await.unwrap();
    assert_eq!(board.chip.as_deref(), Some("3588"));
    assert!(sim.stages().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_loader_for_another_chip_is_reported_as_disagreeing() {
    let sim = SimRockchip::loader(LOCATION, ID, "3588", 1000);
    let file = loader_file(b"8653");
    let loader = Loader::parse(&file).unwrap();
    let mut io = sim.open(LOCATION, &OpenOptions::default()).await.unwrap();
    let board = report(io.as_mut()).await.unwrap();
    assert_eq!(chip_agrees(&loader, &board), Some(false));
}

#[tokio::test(start_paused = true)]
async fn ready_gets_past_one_stall_and_gives_up_after_ten() {
    let sim = SimRockchip::loader(LOCATION, ID, "3588", 1000);
    let mut io = sim.open(LOCATION, &OpenOptions::default()).await.unwrap();

    sim.inject(Fault::Stall);
    ready(io.as_mut()).await.unwrap();

    for _ in 0..10 {
        sim.inject(Fault::Stall);
    }
    let started = Instant::now();
    assert_eq!(ready(io.as_mut()).await, Err(UsbError::Stall));
    // Ten tries, half a second apart: nine waits.
    assert_eq!(started.elapsed(), Duration::from_millis(4500));
}

#[tokio::test(start_paused = true)]
async fn a_stalled_query_is_returned_by_the_report() {
    let sim = SimRockchip::loader(LOCATION, ID, "3588", 1000);
    let mut io = sim.open(LOCATION, &OpenOptions::default()).await.unwrap();
    // The first command's CBW stalls.
    sim.inject(Fault::Stall);
    assert_eq!(report(io.as_mut()).await.err(), Some(UsbError::Stall));
}

#[tokio::test(start_paused = true)]
async fn the_bus_says_maskrom_before_the_usbplug_and_loader_after() {
    let sim = maskrom_board();
    let before = sim.devices().await.unwrap();
    assert_eq!(Mode::of(&before[0]), Mode::Maskrom);
    assert_eq!(before[0].bcd_usb, 0x0200);

    let file = loader_file(b"8853");
    let loader = Loader::parse(&file).unwrap();
    run_usbplug(
        &sim,
        LOCATION,
        &loader,
        &file,
        &OpenOptions::default(),
        REENUMERATE,
    )
    .await
    .unwrap();

    let after = sim.devices().await.unwrap();
    assert_eq!(after[0].location, LOCATION);
    assert_eq!(Mode::of(&after[0]), Mode::Loader);
    assert_eq!(after[0].bcd_device, 0x0100);
}

#[tokio::test(start_paused = true)]
async fn a_loader_board_is_read_as_loader_from_the_start() {
    let sim = SimRockchip::loader(LOCATION, ID, "3588", 1000);
    let devices = sim.devices().await.unwrap();
    assert_eq!(Mode::of(&devices[0]), Mode::Loader);
}
