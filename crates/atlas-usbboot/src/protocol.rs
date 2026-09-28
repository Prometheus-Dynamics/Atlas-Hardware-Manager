//! The two USB boot rounds, independent of any USB library so they can be
//! tested with a scripted device.

use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

use crate::{BootFiles, Chip, UsbBootError};

/// Bulk writes are split into chunks of this size, as `rpiboot` does.
const MAX_BULK: usize = 16 * 1024;
/// `struct file_message { int command; char fname[256]; }`.
const FILE_MESSAGE_LEN: u16 = 4 + 256;
/// Read failures tolerated in a row before the file server gives up.
const MAX_READ_RETRIES: u32 = 10;

/// The three transfers USB boot needs. `UsbBootTransport` implements this
/// over nusb; tests use a scripted fake.
#[async_trait]
pub trait BootTransport: Send {
    /// Vendor OUT request 0 with no data. `wValue`/`wIndex` carry a 32-bit
    /// length: the size of the bulk data that follows, or a file size.
    async fn control_out_len(&mut self, length: u32) -> Result<(), UsbBootError>;
    /// Vendor IN request 0, reading `length` bytes.
    async fn control_in(&mut self, length: u16) -> Result<Vec<u8>, UsbBootError>;
    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbBootError>;
}

/// Progress reported while booting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum BootEvent {
    WaitingForDevice,
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
    /// The Pi asked for a file the boot files do not have. Normal for
    /// optional files.
    FileMissing {
        name: String,
    },
    FileServerDone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileServerOutcome {
    /// The Pi said it has everything.
    Done,
    /// The Pi left the bus, usually because it booted what it was sent.
    Disconnected,
}

fn length_u32(len: usize) -> Result<u32, UsbBootError> {
    u32::try_from(len).map_err(|_| UsbBootError::Protocol(format!("{len} bytes is too large")))
}

async fn ep_write(transport: &mut dyn BootTransport, data: &[u8]) -> Result<(), UsbBootError> {
    transport.control_out_len(length_u32(data.len())?).await?;
    for chunk in data.chunks(MAX_BULK) {
        transport.bulk_out(chunk).await?;
    }
    Ok(())
}

/// Round one: sends the second-stage bootloader to the boot ROM.
pub async fn second_stage(
    transport: &mut dyn BootTransport,
    bootcode: &[u8],
) -> Result<(), UsbBootError> {
    // `boot_message_t { int length; unsigned char signature[20]; }`,
    // unsigned boot: the signature is all zeros.
    let mut message = Vec::with_capacity(24);
    message.extend_from_slice(
        &i32::try_from(bootcode.len())
            .unwrap_or(i32::MAX)
            .to_le_bytes(),
    );
    message.extend_from_slice(&[0u8; 20]);
    ep_write(transport, &message).await?;
    ep_write(transport, bootcode).await?;

    tokio::time::sleep(Duration::from_secs(1)).await;
    match transport.control_in(4).await {
        Ok(reply) => {
            let code = reply
                .get(..4)
                .and_then(|bytes| bytes.try_into().ok())
                .map(i32::from_le_bytes)
                .unwrap_or(0);
            if code == 0 {
                Ok(())
            } else {
                Err(UsbBootError::Protocol(format!(
                    "the boot ROM returned error 0x{code:x} for the second stage"
                )))
            }
        }
        // The ROM may already be restarting into the second stage.
        Err(UsbBootError::Disconnected) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Round two: answers the Pi's file requests until it is done.
pub async fn file_server(
    transport: &mut dyn BootTransport,
    files: &BootFiles,
    chip: Chip,
    on_event: &(dyn Fn(BootEvent) + Send + Sync),
) -> Result<FileServerOutcome, UsbBootError> {
    let mut current: Option<(String, Vec<u8>)> = None;
    let mut failures = 0;
    loop {
        let message = match transport.control_in(FILE_MESSAGE_LEN).await {
            Ok(message) => {
                failures = 0;
                message
            }
            Err(UsbBootError::Disconnected) => return Ok(FileServerOutcome::Disconnected),
            Err(error) => {
                failures += 1;
                if failures >= MAX_READ_RETRIES {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if message.len() < 4 {
            return Err(UsbBootError::Protocol("short file server message".into()));
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
            ep_write(transport, &[]).await?;
            on_event(BootEvent::FileServerDone);
            return Ok(FileServerOutcome::Done);
        }
        // `*NAME*value` messages carry board metadata; acknowledge them.
        if name.starts_with('*') && command != 2 {
            ep_write(transport, &[]).await?;
            continue;
        }

        match command {
            // Get file size.
            0 => match files.read(chip, &name)? {
                Some(data) => {
                    transport.control_out_len(length_u32(data.len())?).await?;
                    on_event(BootEvent::FileRequested {
                        name: name.clone(),
                        bytes: data.len(),
                    });
                    current = Some((name, data));
                }
                None => {
                    current = None;
                    ep_write(transport, &[]).await?;
                    on_event(BootEvent::FileMissing { name });
                }
            },
            // Read file: send what the last size request opened.
            1 => match current.take() {
                Some((opened, data)) if opened == name => {
                    ep_write(transport, &data).await?;
                    on_event(BootEvent::FileSent {
                        name,
                        bytes: data.len(),
                    });
                }
                _ => match files.read(chip, &name)? {
                    Some(data) => {
                        ep_write(transport, &data).await?;
                        on_event(BootEvent::FileSent {
                            name,
                            bytes: data.len(),
                        });
                    }
                    None => {
                        ep_write(transport, &[]).await?;
                        on_event(BootEvent::FileMissing { name });
                    }
                },
            },
            // Done.
            2 => {
                on_event(BootEvent::FileServerDone);
                return Ok(FileServerOutcome::Done);
            }
            other => {
                return Err(UsbBootError::Protocol(format!(
                    "unknown file server command {other}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::bootfiles::tests::{tar, temp_dir};

    #[derive(Debug, PartialEq, Eq)]
    enum Sent {
        Length(u32),
        Bulk(usize),
    }

    /// Replays scripted IN replies and records what the host sent.
    struct Script {
        replies: VecDeque<Result<Vec<u8>, UsbBootError>>,
        sent: Vec<Sent>,
        bulk_bytes: Vec<u8>,
    }

    #[async_trait]
    impl BootTransport for Script {
        async fn control_out_len(&mut self, length: u32) -> Result<(), UsbBootError> {
            self.sent.push(Sent::Length(length));
            Ok(())
        }
        async fn control_in(&mut self, _length: u16) -> Result<Vec<u8>, UsbBootError> {
            self.replies
                .pop_front()
                .unwrap_or(Err(UsbBootError::Disconnected))
        }
        async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbBootError> {
            self.sent.push(Sent::Bulk(data.len()));
            self.bulk_bytes.extend_from_slice(data);
            Ok(())
        }
    }

    fn request(command: i32, name: &str) -> Result<Vec<u8>, UsbBootError> {
        let mut message = command.to_le_bytes().to_vec();
        message.extend_from_slice(name.as_bytes());
        message.resize(FILE_MESSAGE_LEN as usize, 0);
        Ok(message)
    }

    fn boot_files() -> (BootFiles, std::path::PathBuf) {
        let dir = temp_dir("protocol");
        std::fs::write(
            dir.join("bootfiles.bin"),
            tar(&[
                ("2711/bootcode4.bin", &[7u8; 40_000]),
                ("2711/config.txt", b"boot_ramdisk=1"),
            ]),
        )
        .unwrap();
        (BootFiles::open(&dir).unwrap(), dir)
    }

    #[tokio::test(start_paused = true)]
    async fn second_stage_sends_header_then_chunked_bootcode() {
        let mut device = Script {
            replies: VecDeque::from([Ok(0i32.to_le_bytes().to_vec())]),
            sent: Vec::new(),
            bulk_bytes: Vec::new(),
        };

        second_stage(&mut device, &[1u8; 40_000]).await.unwrap();

        assert_eq!(
            device.sent,
            vec![
                Sent::Length(24),
                Sent::Bulk(24),
                Sent::Length(40_000),
                Sent::Bulk(16_384),
                Sent::Bulk(16_384),
                Sent::Bulk(7_232),
            ]
        );
        assert_eq!(&device.bulk_bytes[..4], &40_000i32.to_le_bytes());
    }

    #[tokio::test(start_paused = true)]
    async fn a_nonzero_rom_reply_is_an_error() {
        let mut device = Script {
            replies: VecDeque::from([Ok(5i32.to_le_bytes().to_vec())]),
            sent: Vec::new(),
            bulk_bytes: Vec::new(),
        };
        assert!(matches!(
            second_stage(&mut device, b"x").await,
            Err(UsbBootError::Protocol(_))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn file_server_answers_size_read_missing_and_done() {
        let (files, dir) = boot_files();
        let mut device = Script {
            replies: VecDeque::from([
                request(0, "config.txt"),
                request(1, "config.txt"),
                request(0, "cmdline.txt"),
                request(1, "cmdline.txt"),
                request(0, "*SERIAL*1000abcd"),
                request(2, ""),
            ]),
            sent: Vec::new(),
            bulk_bytes: Vec::new(),
        };
        let events = Mutex::new(Vec::new());

        let outcome = file_server(&mut device, &files, Chip::Bcm2711, &|event| {
            events.lock().unwrap().push(event);
        })
        .await
        .unwrap();

        assert_eq!(outcome, FileServerOutcome::Done);
        assert_eq!(
            device.sent,
            vec![
                Sent::Length(14),
                Sent::Length(14),
                Sent::Bulk(14),
                Sent::Length(0),
                Sent::Length(0),
                Sent::Length(0),
                Sent::Length(0),
            ]
        );
        assert_eq!(device.bulk_bytes, b"boot_ramdisk=1");
        let events = events.into_inner().unwrap();
        assert!(events.contains(&BootEvent::FileMissing {
            name: "cmdline.txt".into()
        }));
        assert_eq!(events.last(), Some(&BootEvent::FileServerDone));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_that_leaves_ends_the_file_server() {
        let (files, dir) = boot_files();
        let mut device = Script {
            replies: VecDeque::from([request(0, "config.txt"), request(1, "config.txt")]),
            sent: Vec::new(),
            bulk_bytes: Vec::new(),
        };

        let outcome = file_server(&mut device, &files, Chip::Bcm2711, &|_| {})
            .await
            .unwrap();

        assert_eq!(outcome, FileServerOutcome::Disconnected);
        let _ = std::fs::remove_dir_all(dir);
    }
}
