//! The fastboot client against a simulated fastboot device on the shared USB
//! bus: getvar, a full download and boot, a refused download, INFO lines,
//! an image over the device's limit, a stall, a timeout, and disconnects
//! mid-download and at boot. Time is paused where a timeout runs.

use std::time::Duration;

use atlas_fastboot::{Fastboot, FastbootError, Variable, find};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, OpenOptions, Timeouts, Transfer, UsbBus, UsbError, UsbId};

const LOCATION: &str = "1-2";
const ID: UsbId = UsbId {
    vendor: 0xf00d,
    product: 0xbeef,
};

fn device(location: &str, product: u16) -> DeviceSummary {
    DeviceSummary {
        location: location.into(),
        vendor: 0xf00d,
        product,
        serial: None,
    }
}

/// A bus with one fastboot device at `LOCATION`, opened.
async fn open(bus: &SimBus) -> Fastboot {
    bus.attach(device(LOCATION, ID.product));
    Fastboot::open(
        bus,
        LOCATION,
        &OpenOptions::default(),
        Duration::from_secs(1),
    )
    .await
    .expect("the device is on the bus")
}

fn reply(text: &str) -> Answer {
    Answer::Data(text.as_bytes().to_vec())
}

/// The `DATA` reply the device sends for an image of `size` bytes.
fn data_reply(size: usize) -> Answer {
    reply(&format!("DATA{size:08x}"))
}

/// The bytes the host wrote, in order.
fn written(bus: &SimBus) -> Vec<Vec<u8>> {
    bus.records(LOCATION)
        .into_iter()
        .filter_map(|record| match record.transfer {
            Transfer::BulkOut(data) => Some(data),
            _ => None,
        })
        .collect()
}

fn image(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn getvar_reads_the_download_limit() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(LOCATION, [Answer::Ok, reply("OKAY0x00100000")]);

    let value = fastboot.getvar(Variable::MaxDownloadSize).await.unwrap();

    assert_eq!(value, "0x00100000");
    let records = bus.records(LOCATION);
    assert_eq!(
        records[0].transfer,
        Transfer::BulkOut(b"getvar:max-download-size".to_vec())
    );
    assert_eq!(records[1].transfer, Transfer::BulkIn { max: 64 });
}

#[tokio::test]
async fn a_full_download_and_boot() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    let size = 65_000;
    let picture = image(size);
    bus.script(
        LOCATION,
        [
            // getvar max-download-size
            Answer::Ok,
            reply("OKAY0x00100000"),
            // download: command, DATA, image, OKAY
            Answer::Ok,
            data_reply(size),
            Answer::Ok,
            reply("OKAY"),
            // boot: command, OKAY
            Answer::Ok,
            reply("OKAY"),
        ],
    );

    fastboot.download(&picture).await.unwrap();
    let info = fastboot.boot().await.unwrap();

    assert!(info.is_empty());
    let sent = written(&bus);
    assert_eq!(sent[0], b"getvar:max-download-size");
    assert_eq!(sent[1], format!("download:{size:08x}").into_bytes());
    assert_eq!(sent[2], picture);
    assert_eq!(sent[3], b"boot");
    assert_eq!(sent.len(), 4, "nothing else was sent");
}

#[tokio::test]
async fn an_image_that_fills_its_last_packet_is_followed_by_a_zero_length_one() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    // 1024 bytes is two full 512-byte packets.
    let size = 1024;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("OKAY0x00100000"),
            Answer::Ok,
            data_reply(size),
            Answer::Ok,
            Answer::Ok,
            reply("OKAY"),
        ],
    );

    fastboot.download(&image(size)).await.unwrap();

    let sent = written(&bus);
    assert_eq!(sent[2].len(), size);
    assert!(sent[3].is_empty(), "the zero-length packet");
}

#[tokio::test]
async fn a_refused_download_is_an_error_and_nothing_boots() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("OKAY0x00100000"),
            Answer::Ok,
            reply("FAILimage rejected by the device"),
        ],
    );

    let error = fastboot.download(&image(16)).await.unwrap_err();

    assert_eq!(
        error,
        FastbootError::Refused {
            command: "download",
            message: "image rejected by the device".into(),
        }
    );
    assert_eq!(fastboot.boot().await.unwrap_err(), FastbootError::NotStaged);
}

#[tokio::test]
async fn a_download_refused_after_its_data_is_an_error() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("OKAY0x00100000"),
            Answer::Ok,
            data_reply(16),
            Answer::Ok,
            reply("FAILwrite error"),
        ],
    );

    let error = fastboot.download(&image(16)).await.unwrap_err();

    assert!(matches!(
        error,
        FastbootError::Refused {
            command: "download",
            ..
        }
    ));
}

#[tokio::test]
async fn info_lines_come_before_the_okay() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("INFOchecking"),
            reply("INFOready"),
            reply("OKAY0x00000100"),
            // download checks the limit first, with its own getvar.
            Answer::Ok,
            reply("OKAY0x00000100"),
            Answer::Ok,
            reply("INFOsending"),
            data_reply(4),
            Answer::Ok,
            reply("INFOwriting"),
            reply("OKAY"),
            Answer::Ok,
            reply("INFObooting kernel"),
            reply("OKAY"),
        ],
    );

    assert_eq!(
        fastboot.getvar(Variable::MaxDownloadSize).await.unwrap(),
        "0x00000100"
    );
    let downloaded = fastboot.download(&image(4)).await.unwrap();
    assert_eq!(downloaded, ["sending", "writing"]);
    let booted = fastboot.boot().await.unwrap();
    assert_eq!(booted, ["booting kernel"]);
}

#[tokio::test]
async fn an_image_over_the_download_limit_is_refused_before_anything_is_sent() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(LOCATION, [Answer::Ok, reply("OKAY0x1000")]);

    let error = fastboot.download(&image(0x2000)).await.unwrap_err();

    assert_eq!(
        error,
        FastbootError::ImageTooLarge {
            size: 0x2000,
            max: 0x1000
        }
    );
    // Only the getvar went out: no download command, no data.
    assert_eq!(bus.records(LOCATION).len(), 2);
}

#[tokio::test]
async fn an_empty_image_is_refused() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;

    assert_eq!(
        fastboot.download(&[]).await.unwrap_err(),
        FastbootError::EmptyImage
    );
    assert!(bus.records(LOCATION).is_empty());
}

#[tokio::test]
async fn a_stall_is_an_error_the_caller_can_retry() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [Answer::Stall, Answer::Ok, reply("OKAY0x00100000")],
    );

    let first = fastboot.getvar(Variable::MaxDownloadSize).await;
    assert_eq!(first, Err(FastbootError::Usb(UsbError::Stall)));

    let second = fastboot.getvar(Variable::MaxDownloadSize).await;
    assert_eq!(second.unwrap(), "0x00100000");
}

#[tokio::test(start_paused = true)]
async fn a_transfer_with_no_answer_times_out() {
    let bus = SimBus::new();
    bus.attach(device(LOCATION, ID.product));
    let options = OpenOptions {
        interface: None,
        timeouts: Timeouts {
            control: Duration::from_secs(1),
            bulk: Duration::from_secs(3),
        },
    };
    let mut fastboot = Fastboot::open(&bus, LOCATION, &options, Duration::from_secs(1))
        .await
        .unwrap();
    bus.script(LOCATION, [Answer::Timeout]);

    let error = fastboot
        .getvar(Variable::MaxDownloadSize)
        .await
        .unwrap_err();

    assert!(matches!(error, FastbootError::Usb(UsbError::Timeout(_))));
}

#[tokio::test]
async fn a_disconnect_mid_download_is_an_error_and_the_image_does_not_boot() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("OKAY0x00100000"),
            Answer::Ok,
            data_reply(16),
            Answer::Disconnect,
        ],
    );

    let error = fastboot.download(&image(16)).await.unwrap_err();

    assert_eq!(error, FastbootError::Usb(UsbError::Disconnected));
    assert!(bus.devices().await.unwrap().is_empty());
    assert_eq!(fastboot.boot().await.unwrap_err(), FastbootError::NotStaged);
}

#[tokio::test]
async fn a_disconnect_before_boot_is_accepted_is_an_error() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("OKAY0x00100000"),
            Answer::Ok,
            data_reply(16),
            Answer::Ok,
            reply("OKAY"),
            // boot: the device leaves the bus before it answers.
            Answer::Ok,
            Answer::Disconnect,
        ],
    );
    fastboot.download(&image(16)).await.unwrap();

    let error = fastboot.boot().await.unwrap_err();

    assert_eq!(error, FastbootError::Usb(UsbError::Disconnected));
}

#[tokio::test]
async fn a_disconnect_after_boot_is_accepted_is_success() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(
        LOCATION,
        [
            Answer::Ok,
            reply("OKAY0x00100000"),
            Answer::Ok,
            data_reply(16),
            Answer::Ok,
            reply("OKAY"),
            Answer::Ok,
            reply("OKAY"),
        ],
    );
    fastboot.download(&image(16)).await.unwrap();

    // The device accepts boot, starts the kernel and re-enumerates: it
    // leaves the bus. boot had already read OKAY, so it is a success.
    let booted = fastboot.boot().await;
    assert!(booted.is_ok());
    assert!(bus.detach(LOCATION), "the device left the bus after OKAY");
    assert!(bus.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn boot_before_download_is_refused_without_sending_anything() {
    let bus = SimBus::new();
    let fastboot = open(&bus).await;

    assert_eq!(fastboot.boot().await.unwrap_err(), FastbootError::NotStaged);
    assert!(bus.records(LOCATION).is_empty());
}

#[tokio::test]
async fn a_reply_too_short_for_its_code_is_a_protocol_error() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    bus.script(LOCATION, [Answer::Ok, reply("OK")]);

    let error = fastboot
        .getvar(Variable::MaxDownloadSize)
        .await
        .unwrap_err();

    assert!(matches!(error, FastbootError::Protocol(_)));
}

#[tokio::test]
async fn an_endless_stream_of_info_lines_is_a_protocol_error() {
    let bus = SimBus::new();
    let mut fastboot = open(&bus).await;
    let mut answers = vec![Answer::Ok];
    answers.extend((0..300).map(|_| reply("INFOstill going")));
    bus.script(LOCATION, answers);

    let error = fastboot
        .getvar(Variable::MaxDownloadSize)
        .await
        .unwrap_err();

    assert!(matches!(error, FastbootError::Protocol(_)));
}

#[tokio::test]
async fn find_matches_only_the_ids_the_caller_gives() {
    let bus = SimBus::new();
    bus.attach(device("1-1", 0x0001));
    bus.attach(device("1-3", ID.product));

    let found = find(&bus, &[ID]).await.unwrap();
    assert_eq!(found.map(|d| d.location), Some("1-3".to_string()));

    let other = UsbId {
        vendor: 0x1,
        product: 0x2,
    };
    assert_eq!(find(&bus, &[other]).await.unwrap(), None);
}
