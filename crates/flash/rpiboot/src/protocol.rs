//! The two boot rounds over a [`UsbIo`], independent of the bus they run on.
//!
//! Round one ([`second_stage`]) sends the second-stage bootloader, then the
//! boot ROM resets. Round two ([`file_server`]) runs on the re-enumerated
//! device: it asks for files by name until it is done or leaves the bus.

use std::time::Duration;

use atlas_usb::{UsbError, UsbIo};

use crate::{BootFiles, Chip, RpiBootError};

/// Bulk writes are split into chunks of this size, as `rpiboot` does.
const MAX_BULK: usize = 16 * 1024;
/// `struct file_message { int command; char fname[256]; }`.
const FILE_MESSAGE_LEN: u16 = 4 + 256;
/// A stalled or timed-out request is retried after this, then after twice as
/// long each time, up to [`MAX_RETRY`]. There is no retry limit: the caller
/// cancels, or the device leaves the bus.
const FIRST_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(5);

/// Progress of a boot, for the app's log and its progress view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootEvent {
    WaitingForDevice,
    /// The boot ROM accepted the second stage.
    SecondStage {
        chip: Chip,
        bytes: usize,
    },
    FileRequested {
        name: String,
        bytes: usize,
    },
    FileSent {
        name: String,
        bytes: usize,
    },
    /// The device asked for a file the boot files do not have. Normal for
    /// optional files.
    FileMissing {
        name: String,
    },
    /// A request stalled or timed out and is being asked again.
    Retried {
        what: String,
    },
    /// A `*NAME*value` message: a fact the bootloader reports, such as the
    /// serial number (needs `recovery_metadata=1` in the recovery config).
    Metadata {
        name: String,
        value: String,
    },
    FileServerDone,
}

/// How round two ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileServerOutcome {
    /// The device said it has everything.
    Done,
    /// The device left the bus, usually because it booted what it was sent.
    Disconnected,
}

/// Splits a `*NAME*value` message into its name and value.
pub(crate) fn parse_metadata(message: &str) -> Option<(String, String)> {
    let (name, value) = message.strip_prefix('*')?.split_once('*')?;
    let name = name.trim();
    (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
}

/// Vendor OUT request 0 with no data. Its value and index carry a 32-bit
/// length: the bulk data that follows, or the size of a file.
async fn send_length(io: &mut dyn UsbIo, length: u32) -> Result<(), RpiBootError> {
    let value = (length & 0xffff) as u16;
    let index = (length >> 16) as u16;
    io.control_out(0, value, index, &[]).await?;
    Ok(())
}

/// Announces `data`'s length, then writes it in chunks.
async fn send(io: &mut dyn UsbIo, data: &[u8]) -> Result<(), RpiBootError> {
    let length = u32::try_from(data.len())
        .map_err(|_| RpiBootError::Protocol(format!("{} bytes is too large", data.len())))?;
    send_length(io, length).await?;
    for chunk in data.chunks(MAX_BULK) {
        io.bulk_out(chunk).await?;
    }
    Ok(())
}

/// Round one: sends the second-stage bootloader to the boot ROM. The ROM may
/// reset as it accepts it, so a device that leaves here is not an error.
pub async fn second_stage(io: &mut dyn UsbIo, bootcode: &[u8]) -> Result<(), RpiBootError> {
    let length = i32::try_from(bootcode.len())
        .map_err(|_| RpiBootError::Protocol("the second stage is too large".into()))?;
    // `boot_message_t { int length; unsigned char signature[20]; }`. An
    // unsigned boot has an all-zero signature.
    let mut header = Vec::with_capacity(24);
    header.extend_from_slice(&length.to_le_bytes());
    header.extend_from_slice(&[0u8; 20]);
    send(io, &header).await?;
    send(io, bootcode).await?;

    tokio::time::sleep(Duration::from_secs(1)).await;
    match io.control_in(0, 4, 0, 4).await {
        Ok(reply) => {
            let bytes: [u8; 4] = reply
                .get(..4)
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or_else(|| {
                    RpiBootError::Protocol(format!(
                        "the boot ROM answered {} bytes to the second stage, not 4",
                        reply.len()
                    ))
                })?;
            match i32::from_le_bytes(bytes) {
                0 => Ok(()),
                code => Err(RpiBootError::RomRefused(code)),
            }
        }
        Err(UsbError::Disconnected) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn check_cancelled(cancelled: &(dyn Fn() -> bool + Send + Sync)) -> Result<(), RpiBootError> {
    if cancelled() {
        Err(RpiBootError::Cancelled)
    } else {
        Ok(())
    }
}

/// Round two: answers the device's file requests until it is done or it
/// leaves the bus, which is the normal end once it has booted what it was
/// sent. A stalled or timed-out request is asked again after a backoff;
/// `cancelled` is checked before each request.
pub async fn file_server(
    io: &mut dyn UsbIo,
    files: &BootFiles,
    chip: Chip,
    on_event: &(dyn Fn(BootEvent) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<FileServerOutcome, RpiBootError> {
    match serve_files(io, files, chip, on_event, cancelled).await {
        Err(RpiBootError::Usb(UsbError::Disconnected)) => Ok(FileServerOutcome::Disconnected),
        other => other,
    }
}

async fn serve_files(
    io: &mut dyn UsbIo,
    files: &BootFiles,
    chip: Chip,
    on_event: &(dyn Fn(BootEvent) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<FileServerOutcome, RpiBootError> {
    let mut current: Option<(String, Vec<u8>)> = None;
    let mut retry = FIRST_RETRY;
    loop {
        check_cancelled(cancelled)?;
        let message = match io
            .control_in(0, FILE_MESSAGE_LEN, 0, FILE_MESSAGE_LEN)
            .await
        {
            Ok(message) => {
                retry = FIRST_RETRY;
                message
            }
            Err(error @ (UsbError::Stall | UsbError::Timeout(_))) => {
                on_event(BootEvent::Retried {
                    what: error.to_string(),
                });
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(MAX_RETRY);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if message.len() < 4 {
            return Err(RpiBootError::Protocol("a short file server message".into()));
        }
        let command = i32::from_le_bytes([message[0], message[1], message[2], message[3]]);
        let name_bytes = &message[4..];
        let end = name_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();

        // An empty name also means done.
        if name.is_empty() {
            send(io, &[]).await?;
            on_event(BootEvent::FileServerDone);
            return Ok(FileServerOutcome::Done);
        }
        // `*NAME*value` messages carry device facts: acknowledge them.
        if name.starts_with('*') && command != 2 {
            send(io, &[]).await?;
            if let Some((fact, value)) = parse_metadata(&name) {
                on_event(BootEvent::Metadata { name: fact, value });
            }
            continue;
        }

        match command {
            // Get file size: the file is opened for the read that follows.
            0 => match files.read(chip, &name)? {
                Some(data) => {
                    send_length(io, length_of(&data)?).await?;
                    on_event(BootEvent::FileRequested {
                        name: name.clone(),
                        bytes: data.len(),
                    });
                    current = Some((name, data));
                }
                None => {
                    current = None;
                    send(io, &[]).await?;
                    on_event(BootEvent::FileMissing { name });
                }
            },
            // Read the file: send what the size request opened.
            1 => match current.take() {
                Some((opened, data)) if opened == name => {
                    send(io, &data).await?;
                    on_event(BootEvent::FileSent {
                        name,
                        bytes: data.len(),
                    });
                }
                _ => match files.read(chip, &name)? {
                    Some(data) => {
                        send(io, &data).await?;
                        on_event(BootEvent::FileSent {
                            name,
                            bytes: data.len(),
                        });
                    }
                    None => {
                        send(io, &[]).await?;
                        on_event(BootEvent::FileMissing { name });
                    }
                },
            },
            2 => {
                on_event(BootEvent::FileServerDone);
                return Ok(FileServerOutcome::Done);
            }
            other => {
                return Err(RpiBootError::Protocol(format!(
                    "unknown file server command {other}"
                )));
            }
        }
    }
}

fn length_of(data: &[u8]) -> Result<u32, RpiBootError> {
    u32::try_from(data.len())
        .map_err(|_| RpiBootError::Protocol(format!("{} bytes is too large", data.len())))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use atlas_usb::sim::{Answer, SimBus};
    use atlas_usb::{DeviceSummary, OpenOptions, Transfer, UsbBus};

    use super::*;

    const LOCATION: &str = "1-2";

    fn device() -> DeviceSummary {
        DeviceSummary {
            location: LOCATION.into(),
            vendor: 0x1234,
            product: 0x0001,
            serial: Some("abc".into()),
        }
    }

    /// A file-server request: command, then the name, padded to 260 bytes.
    fn request(command: i32, name: &str) -> Answer {
        let mut message = command.to_le_bytes().to_vec();
        message.extend_from_slice(name.as_bytes());
        message.resize(usize::from(FILE_MESSAGE_LEN), 0);
        Answer::Data(message)
    }

    fn boot_files() -> BootFiles {
        BootFiles::in_memory([
            ("2712/config.txt", b"boot_ramdisk=1".to_vec()),
            ("2712/bootcode5.bin", vec![7u8; 40_000]),
        ])
    }

    fn never_cancelled() -> bool {
        false
    }

    /// Every transfer the device answered, in order, as the bus recorded
    /// them.
    fn sent(bus: &SimBus) -> Vec<Transfer> {
        bus.records(LOCATION)
            .into_iter()
            .map(|record| record.transfer)
            .collect()
    }

    async fn open(bus: &SimBus) -> Box<dyn UsbIo> {
        bus.open(LOCATION, &OpenOptions::default()).await.unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn second_stage_sends_the_header_then_chunked_bootcode_then_reads_the_reply() {
        let bus = SimBus::new();
        bus.attach(device());
        // Length, header bytes, length, three chunks, then the ROM's reply.
        bus.script(LOCATION, vec![Answer::Ok; 6]);
        bus.script(LOCATION, [Answer::Data(0i32.to_le_bytes().to_vec())]);
        let mut io = open(&bus).await;

        second_stage(io.as_mut(), &[1u8; 40_000]).await.unwrap();

        let mut header = 40_000i32.to_le_bytes().to_vec();
        header.extend_from_slice(&[0u8; 20]);
        assert_eq!(
            sent(&bus),
            vec![
                Transfer::ControlOut {
                    request: 0,
                    value: 24,
                    index: 0,
                    data: vec![]
                },
                Transfer::BulkOut(header),
                Transfer::ControlOut {
                    request: 0,
                    value: (40_000 & 0xffff) as u16,
                    index: 0,
                    data: vec![]
                },
                Transfer::BulkOut(vec![1u8; 16_384]),
                Transfer::BulkOut(vec![1u8; 16_384]),
                Transfer::BulkOut(vec![1u8; 7_232]),
                Transfer::ControlIn {
                    request: 0,
                    value: 4,
                    index: 0,
                    length: 4
                },
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_nonzero_rom_reply_refuses_the_second_stage() {
        let bus = SimBus::new();
        bus.attach(device());
        bus.script(LOCATION, vec![Answer::Ok; 4]);
        bus.script(LOCATION, [Answer::Data(5i32.to_le_bytes().to_vec())]);
        let mut io = open(&bus).await;
        assert_eq!(
            second_stage(io.as_mut(), b"x").await,
            Err(RpiBootError::RomRefused(5))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reset_during_the_second_stage_is_the_normal_path() {
        let bus = SimBus::new();
        bus.attach(device());
        bus.script(LOCATION, vec![Answer::Ok; 4]);
        bus.script(LOCATION, [Answer::Disconnect]);
        let mut io = open(&bus).await;
        assert_eq!(second_stage(io.as_mut(), b"x").await, Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn file_server_answers_size_read_missing_metadata_and_done() {
        let bus = SimBus::new();
        bus.attach(device());
        bus.script(
            LOCATION,
            [
                request(0, "config.txt"),
                Answer::Ok, // size of config.txt
                request(1, "config.txt"),
                Answer::Ok, // data length
                Answer::Ok, // data
                request(0, "cmdline.txt"),
                Answer::Ok, // empty: missing
                request(1, "cmdline.txt"),
                Answer::Ok, // empty: missing again
                request(0, "*SERIAL*1000abcd"),
                Answer::Ok, // acknowledged
                request(2, ""),
                Answer::Ok, // an empty name is acknowledged too
            ],
        );
        let mut io = open(&bus).await;
        let events = Mutex::new(Vec::new());

        let outcome = file_server(
            io.as_mut(),
            &boot_files(),
            Chip::Bcm2712,
            &|event| events.lock().unwrap().push(event),
            &never_cancelled,
        )
        .await
        .unwrap();

        assert_eq!(outcome, FileServerOutcome::Done);
        let events = events.into_inner().unwrap();
        assert!(events.contains(&BootEvent::FileSent {
            name: "config.txt".into(),
            bytes: 14
        }));
        assert!(events.contains(&BootEvent::FileMissing {
            name: "cmdline.txt".into()
        }));
        assert!(events.contains(&BootEvent::Metadata {
            name: "SERIAL".into(),
            value: "1000abcd".into()
        }));
        assert_eq!(events.last(), Some(&BootEvent::FileServerDone));
        // The file's bytes went out in one bulk write.
        assert!(sent(&bus).contains(&Transfer::BulkOut(b"boot_ramdisk=1".to_vec())));
    }

    #[test]
    fn metadata_messages_split_into_name_and_value() {
        assert_eq!(
            parse_metadata("*MAC_ADDR*d8:3a:dd:05:ee:78"),
            Some(("MAC_ADDR".into(), "d8:3a:dd:05:ee:78".into()))
        );
        assert_eq!(parse_metadata("*BAD"), None);
        assert_eq!(parse_metadata("config.txt"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_that_leaves_ends_the_file_server_as_booted() {
        let bus = SimBus::new();
        bus.attach(device());
        bus.script(LOCATION, [request(0, "config.txt"), Answer::Disconnect]);
        let mut io = open(&bus).await;
        let outcome = file_server(
            io.as_mut(),
            &boot_files(),
            Chip::Bcm2712,
            &|_| {},
            &never_cancelled,
        )
        .await;
        assert_eq!(outcome, Ok(FileServerOutcome::Disconnected));
    }

    /// Stalled requests are asked again, with the backoff doubling and capped
    /// at five seconds: 1 + 2 + 4 + 5 + 5 + 5 = 22 s for six stalls.
    #[tokio::test(start_paused = true)]
    async fn stalled_requests_back_off_to_five_seconds_and_then_succeed() {
        let bus = SimBus::new();
        bus.attach(device());
        let mut answers = vec![Answer::Stall; 6];
        answers.push(request(2, ""));
        answers.push(Answer::Ok);
        bus.script(LOCATION, answers);
        let mut io = open(&bus).await;
        let retried = AtomicUsize::new(0);
        let started = tokio::time::Instant::now();

        let outcome = file_server(
            io.as_mut(),
            &boot_files(),
            Chip::Bcm2712,
            &|event| {
                if matches!(event, BootEvent::Retried { .. }) {
                    retried.fetch_add(1, Ordering::SeqCst);
                }
            },
            &never_cancelled,
        )
        .await;

        assert_eq!(outcome, Ok(FileServerOutcome::Done));
        assert_eq!(retried.load(Ordering::SeqCst), 6);
        assert_eq!(started.elapsed(), Duration::from_secs(22));
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_stops_a_device_that_never_answers() {
        let bus = SimBus::new();
        bus.attach(device());
        bus.script(LOCATION, vec![Answer::Stall; 50]);
        let mut io = open(&bus).await;
        let checks = AtomicUsize::new(0);
        let cancelled = || checks.fetch_add(1, Ordering::SeqCst) >= 3;

        let outcome = file_server(
            io.as_mut(),
            &boot_files(),
            Chip::Bcm2712,
            &|_| {},
            &cancelled,
        )
        .await;

        assert_eq!(outcome, Err(RpiBootError::Cancelled));
        // Three stalled requests were asked, then the cancel stopped the loop.
        assert_eq!(bus.records(LOCATION).len(), 3);
    }
}
