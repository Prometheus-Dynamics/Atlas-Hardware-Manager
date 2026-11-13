//! The FEL flow against [`atlas_usb::sim::SimBus`]: every transfer's answer is
//! scripted, so a stall, a drop mid-write, a short write, a timeout or a bad
//! status lands on the exact step the test names. A script with no answer
//! left fails the transfer, so a flow that asks for more than the script
//! gives is caught.
//!
//! The happy path for the same flow is in `src/flash.rs` against
//! `sim::FelDevice`, which keeps the state a read-back needs.

use atlas_fel::flash::{FlashError, flash, load_image, load_spl};
use atlas_fel::layout::{Layout, ThunkIsa};
use atlas_fel::protocol::VERIFY_LEN;
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, OpenOptions, UsbBus, UsbError, UsbIo};

const LOCATION: &str = "1-1";
const SPL_LEN: usize = 0x400;
const IMAGE_LEN: usize = 0x100;
const THUNK_LEN: usize = 284;

/// No swap buffers: the SPL is one piece at its load address.
fn layout() -> Layout {
    Layout {
        thunk_isa: ThunkIsa::Armv7,
        load_address: 0x2000,
        thunk_address: 0x20000,
        swaps: Vec::new(),
        image_address: 0x4100_0000,
        image_entry: 0x4100_0000,
    }
}

fn csw(status: u8) -> Answer {
    let mut block = b"AWUS".to_vec();
    block.extend_from_slice(&[0; 8]);
    block.push(status);
    Answer::Data(block)
}

fn verify_reply(mode: u16) -> Answer {
    let mut reply = vec![0u8; VERIFY_LEN];
    reply[0..8].copy_from_slice(b"AWUSBFEX");
    reply[8..12].copy_from_slice(&0x0733_0000u32.to_le_bytes());
    reply[16..18].copy_from_slice(&mode.to_le_bytes());
    Answer::Data(reply)
}

/// Verify-device: command (3 transfers), then the 32-byte read (3).
fn verify(mode: u16) -> Vec<Answer> {
    vec![
        Answer::Ok,
        Answer::Ok,
        csw(0),
        Answer::Ok,
        verify_reply(mode),
        csw(0),
    ]
}

/// A command with no data stage: request, body, status.
fn command() -> Vec<Answer> {
    vec![Answer::Ok, Answer::Ok, csw(0)]
}

/// A whole download of `len` bytes to an address: command, data stage
/// (request, bytes, a zero-length packet when the bytes fill whole packets,
/// status), then the 8-byte state read.
fn write(len: usize) -> Vec<Answer> {
    let mut steps = command();
    steps.extend([Answer::Ok, Answer::Ok]);
    if len.is_multiple_of(512) {
        steps.push(Answer::Ok);
    }
    steps.push(csw(0));
    steps.extend([
        Answer::Ok,
        Answer::Data(vec![0xff, 0xff, 0, 0, 0, 0, 0, 0]),
        csw(0),
    ]);
    steps
}

/// An upload of `bytes`: command, then request, bytes, status.
fn read(bytes: &[u8]) -> Vec<Answer> {
    let mut steps = command();
    steps.extend([Answer::Ok, Answer::Data(bytes.to_vec()), csw(0)]);
    steps
}

/// The steps that bring a one-piece SPL flow up to the image: verify, the
/// thunk, the SPL, its read-back, the thunk's run, and the return-to-FEL
/// verify.
fn spl_stage(spl: &[u8]) -> Vec<Answer> {
    let mut steps = verify(1);
    steps.extend(write(THUNK_LEN));
    steps.extend(write(spl.len()));
    steps.extend(read(spl));
    steps.extend(command());
    steps.extend(verify(1));
    steps
}

async fn device(bus: &SimBus) -> Box<dyn UsbIo> {
    bus.attach(DeviceSummary {
        location: LOCATION.into(),
        vendor: 0x1f3a,
        product: 0xefe8,
        serial: None,
    });
    bus.open(LOCATION, &OpenOptions::default()).await.unwrap()
}

#[tokio::test]
async fn a_scripted_device_runs_the_whole_flash_and_each_transfer_is_used_once() {
    let spl = vec![0x11u8; SPL_LEN];
    let image = vec![0x22u8; IMAGE_LEN];
    let mut steps = spl_stage(&spl);
    steps.extend(write(IMAGE_LEN));
    steps.extend(read(&image));
    steps.extend(command());
    let expected = steps.len();
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    bus.script(LOCATION, steps);
    let report = flash(io.as_mut(), &layout(), &spl, &image, None)
        .await
        .unwrap();
    assert_eq!(report.spl_pieces, 1);
    assert_eq!(report.image_bytes, IMAGE_LEN);
    // Every scripted answer was asked for, and nothing more.
    assert_eq!(bus.records(LOCATION).len(), expected);
}

#[tokio::test]
async fn the_image_run_that_leaves_the_bus_is_the_expected_end() {
    // A real device leaves FEL when the image starts: the run's status
    // read is a disconnect, and that is success.
    let image = vec![0x22u8; IMAGE_LEN];
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = write(IMAGE_LEN);
    steps.extend(read(&image));
    steps.extend([Answer::Ok, Answer::Ok, Answer::Disconnect]);
    bus.script(LOCATION, steps);
    load_image(io.as_mut(), &layout(), &image).await.unwrap();
    assert!(bus.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_stall_on_the_thunk_data_stage_stops_the_flash() {
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = verify(1);
    steps.extend(command());
    steps.extend([Answer::Ok, Answer::Stall]);
    bus.script(LOCATION, steps);
    let error = load_spl(io.as_mut(), &layout(), &[0u8; SPL_LEN], None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlashError::Usb(UsbError::Stall)),
        "{error:?}"
    );
    // Nothing after the stall was sent.
    assert_eq!(bus.records(LOCATION).len(), 6 + 3 + 2);
}

#[tokio::test]
async fn a_device_that_drops_mid_write_fails_and_leaves_the_bus() {
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = verify(1);
    steps.extend(write(THUNK_LEN));
    // The SPL's write: the command goes through, then the device resets on
    // the data stage.
    steps.extend(command());
    steps.extend([Answer::Ok, Answer::Disconnect]);
    bus.script(LOCATION, steps);
    let error = load_spl(io.as_mut(), &layout(), &[0u8; SPL_LEN], None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlashError::Usb(UsbError::Disconnected)),
        "{error:?}"
    );
    assert!(bus.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_drop_during_the_image_write_is_an_error_not_the_expected_end() {
    // Only the run of the image may end in a disconnect. A drop while the
    // image is written is a failed flash.
    let image = vec![0x22u8; IMAGE_LEN];
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    // The image stage on its own: its write starts at the image's command.
    let mut steps = command();
    steps.extend([Answer::Ok, Answer::Disconnect]);
    bus.script(LOCATION, steps);
    let error = load_image(io.as_mut(), &layout(), &image)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlashError::Usb(UsbError::Disconnected)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_short_write_of_the_thunk_is_reported_with_its_counts() {
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = verify(1);
    steps.extend(command());
    steps.extend([Answer::Ok, Answer::ShortWrite(100)]);
    bus.script(LOCATION, steps);
    let error = load_spl(io.as_mut(), &layout(), &[0u8; SPL_LEN], None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, FlashError::Usb(UsbError::Protocol(text)) if text == &format!("took 100 of {THUNK_LEN} bytes")),
        "{error:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_status_read_that_never_answers_times_out() {
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = verify(1);
    steps.extend([Answer::Ok, Answer::Ok, Answer::Timeout]);
    bus.script(LOCATION, steps);
    let error = load_spl(io.as_mut(), &layout(), &[0u8; SPL_LEN], None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, FlashError::Usb(UsbError::Timeout(_))),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_verify_answer_that_is_not_the_protocol_is_refused() {
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = command();
    steps.extend([Answer::Ok, Answer::Data(vec![0; VERIFY_LEN]), csw(0)]);
    bus.script(LOCATION, steps);
    let error = load_spl(io.as_mut(), &layout(), &[0u8; SPL_LEN], None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, FlashError::Usb(UsbError::Protocol(text)) if text.contains("not AWUSBFEX")),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_failed_command_status_stops_before_its_data() {
    let bus = SimBus::new();
    let mut io = device(&bus).await;
    let mut steps = verify(1);
    steps.extend([Answer::Ok, Answer::Ok, csw(3)]);
    bus.script(LOCATION, steps);
    let error = load_spl(io.as_mut(), &layout(), &[0u8; SPL_LEN], None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, FlashError::Usb(UsbError::Protocol(text)) if text.contains("status byte is 3")),
        "{error:?}"
    );
    assert_eq!(bus.records(LOCATION).len(), 6 + 3);
}
