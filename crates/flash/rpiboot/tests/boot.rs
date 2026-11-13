//! Both boot rounds on the simulated bus: a reset between the rounds, a
//! stalled request, a missing file, cancelling, and a device that never comes.
//! The device's answers are the protocol's own: the script is what a boot ROM
//! and its file server would answer, in order.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use atlas_rpiboot::{
    BootEvent, BootFiles, BootSettings, Chip, FileServerOutcome, RpiBootError, boot_device,
};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, OpenOptions, Timeouts, UsbBus, UsbError};

const LOCATION: &str = "1-2";

fn device() -> DeviceSummary {
    DeviceSummary {
        location: LOCATION.into(),
        vendor: 0x0a5c,
        product: 0x2712,
        serial: None,
        bcd_usb: 0,
        bcd_device: 0,
    }
}

fn request(command: i32, name: &str) -> Answer {
    let mut message = command.to_le_bytes().to_vec();
    message.extend_from_slice(name.as_bytes());
    message.resize(260, 0);
    Answer::Data(message)
}

/// The round one answers: the second stage's six writes, then the reset.
fn first_round() -> Vec<Answer> {
    let mut answers = vec![Answer::Ok; 6];
    answers.push(Answer::Disconnect);
    answers
}

fn flasher() -> BootFiles {
    BootFiles::in_memory([
        ("2712/bootcode5.bin", vec![9u8; 40_000]),
        ("config.txt", b"boot_ramdisk=1".to_vec()),
        ("boot.img", vec![3u8; 100]),
    ])
}

fn settings() -> BootSettings {
    BootSettings {
        open: OpenOptions {
            interface: None,
            timeouts: Timeouts::default(),
        },
        appear_timeout: Duration::from_secs(30),
    }
}

fn never_cancelled() -> bool {
    false
}

/// Attaches the device again, as the reset brings it back at the same
/// location, with the round two answers queued.
fn re_enumerate(bus: &SimBus, answers: Vec<Answer>) {
    bus.attach(device());
    bus.script(LOCATION, answers);
}

#[tokio::test(start_paused = true)]
async fn both_rounds_boot_through_a_reset_and_serve_the_files() {
    let events = Mutex::new(Vec::new());
    let files = flasher();
    let cfg = settings();
    let never = never_cancelled;
    let record = |event: BootEvent| events.lock().unwrap().push(event);
    let bus = SimBus::new();
    bus.attach(device());
    bus.script(LOCATION, first_round());
    let started = tokio::time::Instant::now();

    let (outcome, ()) = tokio::join!(
        boot_device(&bus, LOCATION, Chip::Bcm2712, &files, &cfg, &record, &never,),
        async {
            // The device is gone for two seconds, then comes back.
            tokio::time::sleep(Duration::from_secs(2)).await;
            re_enumerate(
                &bus,
                vec![
                    request(0, "config.txt"),
                    Answer::Ok,
                    request(1, "config.txt"),
                    Answer::Ok,
                    Answer::Ok,
                    request(0, "boot.img"),
                    Answer::Ok,
                    request(1, "boot.img"),
                    Answer::Ok,
                    Answer::Ok,
                    request(2, ""),
                    Answer::Ok,
                ],
            );
        }
    );

    assert_eq!(outcome, Ok(FileServerOutcome::Done));
    // Round two could only start once the device was back: the second round
    // waited out the reset, so the boot took at least the two seconds away.
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(2), "waited {waited:?}");
    let events = events.into_inner().unwrap();
    assert_eq!(events[0], BootEvent::WaitingForDevice);
    assert!(events.contains(&BootEvent::SecondStage {
        chip: Chip::Bcm2712,
        bytes: 40_000
    }));
    assert!(events.contains(&BootEvent::FileSent {
        name: "boot.img".into(),
        bytes: 100
    }));
    assert_eq!(events.last(), Some(&BootEvent::FileServerDone));
}

#[tokio::test(start_paused = true)]
async fn a_stalled_request_is_asked_again_and_the_boot_finishes() {
    let events = Mutex::new(Vec::new());
    let files = flasher();
    let cfg = settings();
    let never = never_cancelled;
    let record = |event: BootEvent| events.lock().unwrap().push(event);
    let bus = SimBus::new();
    bus.attach(device());
    bus.script(LOCATION, first_round());

    let (outcome, ()) = tokio::join!(
        boot_device(&bus, LOCATION, Chip::Bcm2712, &files, &cfg, &record, &never,),
        async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            re_enumerate(
                &bus,
                vec![
                    Answer::Stall,
                    Answer::Stall,
                    request(0, "config.txt"),
                    Answer::Ok,
                    request(1, "config.txt"),
                    Answer::Ok,
                    Answer::Ok,
                    request(2, ""),
                    Answer::Ok,
                ],
            );
        }
    );

    assert_eq!(outcome, Ok(FileServerOutcome::Done));
    let events = events.into_inner().unwrap();
    let retries = events
        .iter()
        .filter(|event| matches!(event, BootEvent::Retried { .. }))
        .count();
    assert_eq!(retries, 2);
    assert!(events.contains(&BootEvent::FileSent {
        name: "config.txt".into(),
        bytes: 14
    }));
}

#[tokio::test(start_paused = true)]
async fn a_file_the_boot_files_lack_is_reported_and_the_boot_goes_on() {
    let events = Mutex::new(Vec::new());
    let files = flasher();
    let cfg = settings();
    let never = never_cancelled;
    let record = |event: BootEvent| events.lock().unwrap().push(event);
    let bus = SimBus::new();
    bus.attach(device());
    bus.script(LOCATION, first_round());

    let (outcome, ()) = tokio::join!(
        boot_device(&bus, LOCATION, Chip::Bcm2712, &files, &cfg, &record, &never,),
        async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            re_enumerate(
                &bus,
                vec![
                    request(0, "cmdline.txt"),
                    Answer::Ok,
                    request(2, ""),
                    Answer::Ok,
                ],
            );
        }
    );

    assert_eq!(outcome, Ok(FileServerOutcome::Done));
    assert!(
        events
            .into_inner()
            .unwrap()
            .contains(&BootEvent::FileMissing {
                name: "cmdline.txt".into()
            })
    );
}

#[tokio::test(start_paused = true)]
async fn a_device_that_resets_again_ends_the_boot_as_booted() {
    let files = flasher();
    let cfg = settings();
    let never = never_cancelled;
    let ignore = |_: BootEvent| {};
    let bus = SimBus::new();
    bus.attach(device());
    bus.script(LOCATION, first_round());

    let (outcome, ()) = tokio::join!(
        boot_device(&bus, LOCATION, Chip::Bcm2712, &files, &cfg, &ignore, &never,),
        async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            re_enumerate(&bus, vec![request(0, "config.txt"), Answer::Disconnect]);
        }
    );

    assert_eq!(outcome, Ok(FileServerOutcome::Disconnected));
}

#[tokio::test(start_paused = true)]
async fn cancelling_between_the_rounds_touches_nothing_more() {
    let files = flasher();
    let cfg = settings();
    let ignore = |_: BootEvent| {};
    let bus = SimBus::new();
    bus.attach(device());
    bus.script(LOCATION, first_round());
    let checks = AtomicUsize::new(0);
    // Not cancelled before the first round, cancelled before the second.
    let cancelled = || checks.fetch_add(1, Ordering::SeqCst) >= 1;

    let outcome = boot_device(
        &bus,
        LOCATION,
        Chip::Bcm2712,
        &files,
        &cfg,
        &ignore,
        &cancelled,
    )
    .await;

    assert_eq!(outcome, Err(RpiBootError::Cancelled));
    assert_eq!(bus.devices().await.unwrap(), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn a_second_stage_that_is_missing_is_refused_before_the_device_is_touched() {
    let cfg = settings();
    let never = never_cancelled;
    let ignore = |_: BootEvent| {};
    let bus = SimBus::new();
    bus.attach(device());
    let files = BootFiles::in_memory([("config.txt", b"x".to_vec())]);

    let outcome = boot_device(&bus, LOCATION, Chip::Bcm2711, &files, &cfg, &ignore, &never).await;

    assert!(matches!(outcome, Err(RpiBootError::MissingFile { .. })));
    assert!(bus.records(LOCATION).is_empty());
}

#[tokio::test(start_paused = true)]
async fn no_device_within_the_appear_timeout_is_a_timeout() {
    let files = flasher();
    let never = never_cancelled;
    let ignore = |_: BootEvent| {};
    let bus = SimBus::new();
    let mut quick = settings();
    quick.appear_timeout = Duration::from_secs(3);

    let outcome = boot_device(
        &bus,
        LOCATION,
        Chip::Bcm2712,
        &files,
        &quick,
        &ignore,
        &never,
    )
    .await;

    assert!(matches!(
        outcome,
        Err(RpiBootError::Usb(UsbError::Timeout(_)))
    ));
}
