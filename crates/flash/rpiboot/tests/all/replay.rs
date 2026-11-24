//! Replays of two real BCM2712 second stages against atlas-rpiboot (D19).
//!
//! The request sequences come from verbose logs of Raspberry Pi's `rpiboot`
//! (usbboot 20251209~095558, faf4fb9d) booting Raze #2, a CM5, on 2026-11-22.
//! Both logs are kept in the bench session directory:
//! - `a.log`: a boot partition of exactly the bay (BAY-EXACT): `config.txt`,
//!   then `boot.img`, then Done;
//! - `a0.log`: the same boot as a tree: the bootloader reads config, the bay's
//!   device tree, overlays, the kernel and the initramfs, with three files it
//!   does not find.
//!
//! Each log line `Received message <GetFileSize|ReadFile|Done>: <name>` is one
//! row below, with the size the log printed (`File size = N bytes` or
//! `libusb_bulk_transfer sent N bytes`), or `Missing` where it printed `Cannot
//! open file`. The simulated device asks exactly these, in this order. The
//! board was put into USB boot by its EEPROM, so it came up as the file server
//! (`Found serial number 4`): no second stage is sent.
//!
//! The test asserts what went over the bus, transfer by transfer, against what
//! `rpiboot`'s `file_server()` sends for the same request: a size as a vendor
//! OUT request 0 with the size in value and index; a missing file as that
//! request with length 0 and no data; a file as its length, then bulk writes of
//! at most 16 KiB; nothing after Done.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use atlas_rpiboot::{BootEvent, BootFiles, BootSettings, Chip, FileServerOutcome, boot_device};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, OpenOptions, Timeouts, Transfer, UsbId};

const LOCATION: &str = "3-4.2.1.4.4.2";
const BCM2712: UsbId = UsbId {
    vendor: 0x0a5c,
    product: 0x2712,
};
/// `libusb` transfers in `rpiboot` are at most this long (`LIBUSB_MAX_TRANSFER`).
const MAX_BULK: usize = 16 * 1024;
/// `sizeof(struct file_message)`: a 4-byte command and a 256-byte name.
const FILE_MESSAGE: u16 = 260;

#[derive(Clone, Copy, Debug)]
enum Row {
    Size(&'static str, usize),
    Read(&'static str, usize),
    Missing(&'static str),
    Done(&'static str),
}
use Row::{Done, Missing, Read, Size};

/// `a.log`, the BAY-EXACT boot.
const A_LOG: &[Row] = &[
    Size("config.txt", 15),
    Read("config.txt", 15),
    Size("boot.img", 82_837_504),
    Read("boot.img", 82_837_504),
    Done("boot.img"),
];

/// `a0.log`, the tree boot.
const A0_LOG: &[Row] = &[
    Size("config.txt", 1702),
    Read("config.txt", 1702),
    Size("default.txt", 1024),
    Read("default.txt", 1024),
    Missing("pieeprom.sig"),
    Size("slots/bay0/bcm2712-rpi-cm5-cm5io.dtb", 79_751),
    Read("slots/bay0/bcm2712-rpi-cm5-cm5io.dtb", 79_751),
    Size("/config.txt", 1702),
    Read("/config.txt", 1702),
    Size("/default.txt", 1024),
    Read("/default.txt", 1024),
    Size("/previous.txt", 1024),
    Read("/previous.txt", 1024),
    Size("/config.txt", 1702),
    Read("/config.txt", 1702),
    Size("/default.txt", 1024),
    Read("/default.txt", 1024),
    Size("slots/bay0/overlays/README", 161),
    Missing("slots/bay0/kernel_2712.img"),
    Size("slots/bay0/kernel8.img", 6_912_556),
    Size("slots/bay0/initramfs.cpio", 661_128),
    Read("slots/bay0/initramfs.cpio", 661_128),
    Size("slots/bay0/bcm2712-rpi-cm5-cm5io.dtb", 79_751),
    Read("slots/bay0/bcm2712-rpi-cm5-cm5io.dtb", 79_751),
    Size("/slots/bay0/overlays/overlay_map.dtb", 6331),
    Read("/slots/bay0/overlays/overlay_map.dtb", 6331),
    Size("/slots/bay0/overlays/bcm2712d0.dtbo", 1363),
    Read("/slots/bay0/overlays/bcm2712d0.dtbo", 1363),
    Size("/config.txt", 1702),
    Read("/config.txt", 1702),
    Size("/default.txt", 1024),
    Read("/default.txt", 1024),
    Size("/slots/bay0/overlays/lemnos-board.dtbo", 1884),
    Read("/slots/bay0/overlays/lemnos-board.dtbo", 1884),
    Size("/slots/bay0/overlays/vc4-kms-v3d-pi5.dtbo", 3306),
    Read("/slots/bay0/overlays/vc4-kms-v3d-pi5.dtbo", 3306),
    Size("/slots/bay0/overlays/ramoops-pi5.dtbo", 745),
    Read("/slots/bay0/overlays/ramoops-pi5.dtbo", 745),
    Size("/slots/bay0/cmdline.txt", 370),
    Read("/slots/bay0/cmdline.txt", 370),
    Missing("slots/bay0/armstub8-2712.bin"),
    Size("slots/bay0/kernel8.img", 6_912_556),
    Read("slots/bay0/kernel8.img", 6_912_556),
    Done("slots/bay0/kernel8.img"),
];

/// A file's bytes: its size, and a pattern of its own so one file sent for
/// another shows.
fn contents(name: &str, size: usize) -> Vec<u8> {
    let seed = name
        .bytes()
        .fold(7u8, |acc, byte| acc.wrapping_mul(31) ^ byte);
    (0..size)
        .map(|at| seed.wrapping_add((at % 251) as u8))
        .collect()
}

/// The served folder as the log saw it: every file it found, at its size, keyed
/// by its path below the folder; the second stage in `2712/`, as in serve-A.
fn served(log: &[Row]) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        "2712/bootcode5.bin".to_string(),
        contents("bootcode5", 4096),
    );
    for row in log {
        if let Size(name, size) | Read(name, size) = *row {
            let key = name.trim_start_matches('/').to_string();
            let data = contents(&key, size);
            if let Some(known) = files.insert(key.clone(), data) {
                assert_eq!(known.len(), size, "{key}: the log gave two sizes");
            }
        }
    }
    files
}

fn request(command: i32, name: &str) -> Answer {
    let mut message = command.to_le_bytes().to_vec();
    message.extend_from_slice(name.as_bytes());
    message.resize(usize::from(FILE_MESSAGE), 0);
    Answer::Data(message)
}

/// The device's side: each request, then an accept for every transfer the host
/// makes in answer.
fn script(log: &[Row]) -> Vec<Answer> {
    let mut answers = Vec::new();
    for row in log {
        match *row {
            Size(name, _) => answers.extend([request(0, name), Answer::Ok]),
            Missing(name) => answers.extend([request(0, name), Answer::Ok]),
            Read(name, size) => {
                answers.extend([request(1, name), Answer::Ok]);
                answers.extend(std::iter::repeat_n(Answer::Ok, size.div_ceil(MAX_BULK)));
            }
            Done(name) => answers.push(request(2, name)),
        }
    }
    answers
}

/// One host answer, as `rpiboot`'s `file_server()` gives it.
#[derive(Debug, PartialEq, Eq)]
enum Sent {
    /// `ep_read`: the request is read with a vendor IN request 0 of 260 bytes.
    Asked,
    /// `libusb_control_transfer(VENDOR, 0, n & 0xffff, n >> 16)`: a size, or
    /// the length of the data that follows.
    Length(usize),
    /// `ep_write`'s bulk writes of one file, joined: chunk lengths and bytes.
    Data { chunks: Vec<usize>, bytes: Vec<u8> },
}

fn expected(log: &[Row], files: &BTreeMap<String, Vec<u8>>) -> Vec<Sent> {
    let mut sent = Vec::new();
    for row in log {
        sent.push(Sent::Asked);
        match *row {
            Size(_, size) => sent.push(Sent::Length(size)),
            Missing(_) => sent.push(Sent::Length(0)),
            Read(name, size) => {
                let bytes = files[name.trim_start_matches('/')].clone();
                assert_eq!(bytes.len(), size);
                let chunks = bytes.chunks(MAX_BULK).map(<[u8]>::len).collect();
                sent.push(Sent::Length(size));
                sent.push(Sent::Data { chunks, bytes });
            }
            Done(_) => {}
        }
    }
    sent
}

fn observed(bus: &SimBus) -> Vec<Sent> {
    let mut sent: Vec<Sent> = Vec::new();
    for record in bus.records(LOCATION) {
        match record.transfer {
            Transfer::ControlIn {
                request: 0,
                value,
                index: 0,
                length,
            } if value == FILE_MESSAGE && length == FILE_MESSAGE => sent.push(Sent::Asked),
            Transfer::ControlOut {
                request: 0,
                value,
                index,
                data,
            } if data.is_empty() => {
                sent.push(Sent::Length(usize::from(value) | usize::from(index) << 16))
            }
            Transfer::BulkOut(data) => match sent.last_mut() {
                Some(Sent::Data { chunks, bytes }) => {
                    chunks.push(data.len());
                    bytes.extend_from_slice(&data);
                }
                _ => sent.push(Sent::Data {
                    chunks: vec![data.len()],
                    bytes: data,
                }),
            },
            other => panic!("rpiboot's file server sends no {other:?}"),
        }
    }
    sent
}

/// Runs one replay and checks the bus, the events and the end.
async fn replay(log: &[Row]) {
    let files = served(log);
    let boot_files = BootFiles::in_memory(files.clone());
    let bus = SimBus::new();
    bus.attach(DeviceSummary::new(
        LOCATION,
        BCM2712.vendor,
        BCM2712.product,
    ));
    // `Found serial number 4`: the EEPROM's file server, not the boot ROM.
    assert!(bus.set_serial_index(LOCATION, 4));
    bus.script(LOCATION, script(log));
    let events = Mutex::new(Vec::new());
    let settings = BootSettings {
        open: OpenOptions {
            interface: None,
            timeouts: Timeouts::default(),
        },
        appear_timeout: Duration::from_secs(30),
    };

    let outcome = boot_device(
        &bus,
        LOCATION,
        BCM2712,
        Chip::Bcm2712,
        &boot_files,
        &settings,
        &|event| events.lock().unwrap().push(event),
        &|| false,
    )
    .await;

    assert_eq!(outcome, Ok(FileServerOutcome::Done));
    let observed = observed(&bus);
    let expected = expected(log, &files);
    assert_eq!(observed.len(), expected.len(), "transfer count");
    for (at, (seen, wanted)) in observed.iter().zip(&expected).enumerate() {
        assert!(seen == wanted, "answer {at} differs from rpiboot's");
    }
    // Every script answer was used: the boot ended where the log ends.
    assert!(bus.records(LOCATION).len() >= script(log).len() - 1);

    let events = events.into_inner().unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, BootEvent::SecondStage { .. })),
        "a file server was sent the second stage"
    );
    let missing: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            BootEvent::FileMissing { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let logged_missing: Vec<&str> = log
        .iter()
        .filter_map(|row| match row {
            Missing(name) => Some(*name),
            _ => None,
        })
        .collect();
    assert_eq!(missing, logged_missing);
    let sent: Vec<(String, usize)> = events
        .iter()
        .filter_map(|event| match event {
            BootEvent::FileSent { name, bytes } => Some((name.clone(), *bytes)),
            _ => None,
        })
        .collect();
    let logged_sent: Vec<(String, usize)> = log
        .iter()
        .filter_map(|row| match row {
            Read(name, size) => Some(((*name).to_string(), *size)),
            _ => None,
        })
        .collect();
    assert_eq!(sent, logged_sent);
    assert_eq!(events.last(), Some(&BootEvent::FileServerDone));
}

#[tokio::test(start_paused = true)]
async fn the_bay_exact_boot_is_served_as_rpiboot_served_it() {
    replay(A_LOG).await;
}

#[tokio::test(start_paused = true)]
async fn the_tree_boot_is_served_as_rpiboot_served_it() {
    replay(A0_LOG).await;
}

/// `rpiboot` sends the second stage to iSerialNumber 0 and 3 only.
#[tokio::test(start_paused = true)]
async fn a_boot_rom_still_gets_the_second_stage_first() {
    for index in [0u8, 3] {
        let bus = SimBus::new();
        bus.attach(DeviceSummary::new(
            LOCATION,
            BCM2712.vendor,
            BCM2712.product,
        ));
        assert!(bus.set_serial_index(LOCATION, index));
        // Round one: the boot message's length and bytes, the second stage's
        // length and its one chunk, then the ROM resets as it is asked for
        // its reply.
        let mut answers = vec![Answer::Ok; 4];
        answers.push(Answer::Disconnect);
        bus.script(LOCATION, answers);
        let files = BootFiles::in_memory(served(A_LOG));
        let settings = BootSettings {
            open: OpenOptions::default(),
            appear_timeout: Duration::from_secs(1),
        };
        let events = Mutex::new(Vec::new());
        let _ = boot_device(
            &bus,
            LOCATION,
            BCM2712,
            Chip::Bcm2712,
            &files,
            &settings,
            &|event| events.lock().unwrap().push(event),
            &|| false,
        )
        .await;
        // The ROM took the 4096-byte second stage and reset: round two then
        // finds no device within the second, which this test does not need.
        assert!(
            events
                .into_inner()
                .unwrap()
                .contains(&BootEvent::SecondStage {
                    chip: Chip::Bcm2712,
                    bytes: 4096
                }),
            "index {index}: no second stage"
        );
    }
}
