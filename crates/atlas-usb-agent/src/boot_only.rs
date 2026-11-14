//! A boot-only job: USB boots the Pi at a port with a set of boot files and
//! does nothing else. The job never looks at a disk on its port, never
//! opens a block device and writes nothing. It runs the boot ROM's two
//! rounds (the second stage, then the file server loop) through
//! atlas-usbboot, reports each file the Pi asked for, and ends when the
//! loop ends, at a timeout, or on an error.
//!
//! One-shot: once the file server loop has ended, the job answers no more
//! boot-ROM appearances. It looks once more at its port, right away, and
//! reports a boot ROM it finds there as not served.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_usb_agent::{BootOutcome, Event, UsbStep};
use atlas_usbboot::{
    BROADCOM_VENDOR_ID, BootEvent, BootFiles, BootTransport, Chip, FileServerOutcome, UsbBootError,
    UsbBootTransport, file_server, second_stage,
};
use sha2::{Digest, Sha256};

/// The serial indexes the boot ROM shows in round one (the second stage
/// goes to a device with one of these); the file server's round shows
/// another.
const ROM_INDEXES: [u8; 2] = [0, 3];
/// Rounds before giving up: two are normal; older chips can take more.
const MAX_ROUNDS: u8 = 6;
const POLL: Duration = Duration::from_millis(250);

/// A Pi that came to USB boot at the port.
pub(crate) struct Appearance {
    pub chip: Chip,
    /// Its serial index: a boot ROM's round (0 or 3), or the file server's.
    pub index: u8,
    pub serial: Option<String>,
    pub transport: Box<dyn BootTransport>,
}

/// What the job may see of its port: the Pis that come to USB boot there.
/// Nothing else of the port is reachable from here.
pub(crate) trait Port {
    /// The next Pi at the port whose serial index differs from `previous`,
    /// waiting up to `wait` (`Duration::ZERO` looks once). `None` when none
    /// came.
    fn appearance(
        &mut self,
        previous: Option<u8>,
        wait: Duration,
        cancel: &AtomicBool,
    ) -> impl Future<Output = Result<Option<Appearance>, UsbBootError>> + Send;
}

/// The real port: Pis in USB boot at one USB port, over nusb.
pub(crate) struct UsbPort {
    /// nusb's location form: `3-1.1` is `003-1.1`.
    location: String,
}

impl UsbPort {
    /// The port (sysfs form, `3-1.1`), or `None` for a port that isn't one.
    pub(crate) fn at(port: &str) -> Option<Self> {
        let (bus, ports) = port.split_once('-')?;
        Some(Self {
            location: format!("{:03}-{ports}", bus.parse::<u16>().ok()?),
        })
    }
}

fn location(info: &nusb::DeviceInfo) -> String {
    let ports: Vec<String> = info.port_chain().iter().map(u8::to_string).collect();
    format!("{}-{}", info.bus_id(), ports.join("."))
}

impl Port for UsbPort {
    async fn appearance(
        &mut self,
        previous: Option<u8>,
        wait: Duration,
        cancel: &AtomicBool,
    ) -> Result<Option<Appearance>, UsbBootError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(UsbBootError::Cancelled);
            }
            // Collected before any await: the list is not held across one.
            let boot_roms: Vec<(nusb::DeviceInfo, Chip)> = nusb::list_devices()
                .await
                .map_err(|error| UsbBootError::Usb(error.to_string()))?
                .filter_map(|info| {
                    (info.vendor_id() == BROADCOM_VENDOR_ID)
                        .then(|| Chip::from_product_id(info.product_id()))
                        .flatten()
                        .map(|chip| (info, chip))
                })
                .filter(|(info, _)| location(info) == self.location)
                .collect();
            for (info, chip) in boot_roms {
                match UsbBootTransport::open(&info).await {
                    Ok((transport, index)) if Some(index) != previous => {
                        let serial = info
                            .serial_number()
                            .map(str::trim)
                            .filter(|serial| !serial.is_empty())
                            .map(str::to_string);
                        return Ok(Some(Appearance {
                            chip,
                            index,
                            serial,
                            transport: Box::new(transport),
                        }));
                    }
                    Ok(_) => {}
                    // Access problems will not fix themselves; report them now.
                    Err(error @ UsbBootError::Access { .. }) => return Err(error),
                    Err(_) => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

/// Runs one boot-only job at `port` with `files`, reporting to `sink`, and
/// returns the job's last event (`BootEnd`). `timeout` bounds each wait for
/// a Pi to come to USB boot.
pub(crate) async fn run<P: Port>(
    port: &mut P,
    files: &BootFiles,
    timeout: Duration,
    cancel: &AtomicBool,
    sink: &(dyn Fn(Event) + Send + Sync),
) -> Event {
    let mut previous = None;
    for _ in 0..MAX_ROUNDS {
        sink(Event::UsbStep {
            step: UsbStep::Waiting,
        });
        let mut appearance = match port.appearance(previous, timeout, cancel).await {
            Ok(Some(appearance)) => appearance,
            Ok(None) => {
                return end(
                    BootOutcome::Timeout,
                    Some(format!(
                        "no Pi came to USB boot within {} s",
                        timeout.as_secs()
                    )),
                );
            }
            Err(error) => return failed(error),
        };
        previous = Some(appearance.index);

        if ROM_INDEXES.contains(&appearance.index) {
            // Round one: the boot ROM takes the second stage.
            let bootcode = match files.second_stage(appearance.chip) {
                Ok(bootcode) => bootcode,
                Err(error) => return failed(error),
            };
            sink(Event::UsbStep {
                step: UsbStep::SecondStage {
                    chip: appearance.chip.label().into(),
                    bytes: bootcode.len() as u64,
                },
            });
            if let Err(error) = second_stage(&mut *appearance.transport, &bootcode).await {
                return failed(error);
            }
            continue;
        }

        // Round two: the file server loop, until the Pi is done or leaves.
        let chip = appearance.chip;
        let loop_end = file_server(&mut *appearance.transport, files, chip, &|event| {
            report(event, files, chip, sink);
        })
        .await;
        drop(appearance);
        let outcome = match loop_end {
            Ok(FileServerOutcome::Done) => BootOutcome::Done,
            Ok(FileServerOutcome::Disconnected) => BootOutcome::Disconnected,
            Err(error) => return failed(error),
        };

        // One shot: the loop has ended, so nothing more is answered here. A
        // boot ROM already back at the port is left alone and reported.
        if let Ok(Some(again)) = port.appearance(previous, Duration::ZERO, cancel).await
            && ROM_INDEXES.contains(&again.index)
        {
            sink(Event::UsbStep {
                step: UsbStep::NotServed {
                    reason: "one-shot".into(),
                    serial: again.serial,
                },
            });
        }
        return end(outcome, None);
    }
    end(
        BootOutcome::Error,
        Some(format!(
            "the Pi kept asking for its second stage in {MAX_ROUNDS} rounds; check the boot files match the board"
        )),
    )
}

/// One file-server event as a job event. A file served is hashed as it was
/// sent (the same lookup the protocol made).
fn report(event: BootEvent, files: &BootFiles, chip: Chip, sink: &(dyn Fn(Event) + Send + Sync)) {
    match event {
        BootEvent::FileSent { name, bytes } => {
            let sha256 = files
                .read(chip, &name)
                .ok()
                .flatten()
                .map(|data| hex(&Sha256::digest(&data)));
            sink(Event::BootFile {
                name,
                served: true,
                bytes: Some(bytes as u64),
                sha256,
            });
        }
        BootEvent::FileMissing { name } => sink(Event::BootFile {
            name,
            served: false,
            bytes: None,
            sha256: None,
        }),
        BootEvent::Metadata { name, value } => sink(Event::UsbStep {
            step: UsbStep::Metadata { name, value },
        }),
        BootEvent::WaitingForDevice
        | BootEvent::SecondStage { .. }
        | BootEvent::FileRequested { .. }
        | BootEvent::FileServerDone => {}
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn end(outcome: BootOutcome, message: Option<String>) -> Event {
    Event::BootEnd { outcome, message }
}

fn failed(error: UsbBootError) -> Event {
    let outcome = match error {
        UsbBootError::Cancelled => BootOutcome::Cancelled,
        _ => BootOutcome::Error,
    };
    let message = match error.fix() {
        Some(fix) => format!("{error}. {fix}"),
        None => error.to_string(),
    };
    end(outcome, Some(message))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;

    /// One USB transport: the Pi's replies, and what the host sent back.
    struct Wire {
        replies: VecDeque<Result<Vec<u8>, UsbBootError>>,
        sent: Arc<Mutex<Vec<u8>>>,
    }

    #[async_trait]
    impl BootTransport for Wire {
        async fn control_out_len(&mut self, _length: u32) -> Result<(), UsbBootError> {
            Ok(())
        }
        async fn control_in(&mut self, _length: u16) -> Result<Vec<u8>, UsbBootError> {
            self.replies
                .pop_front()
                .unwrap_or(Err(UsbBootError::Disconnected))
        }
        async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbBootError> {
            self.sent.lock().unwrap().extend_from_slice(data);
            Ok(())
        }
    }

    /// A file server request: the command and the name, as the Pi sends it.
    fn request(command: i32, name: &str) -> Result<Vec<u8>, UsbBootError> {
        let mut message = command.to_le_bytes().to_vec();
        message.extend_from_slice(name.as_bytes());
        message.resize(4 + 256, 0);
        Ok(message)
    }

    /// The boot ROM's answer to the second stage: success.
    fn second_stage_ok() -> Result<Vec<u8>, UsbBootError> {
        Ok(0i32.to_le_bytes().to_vec())
    }

    fn appearance(index: u8, replies: Vec<Result<Vec<u8>, UsbBootError>>) -> Appearance {
        Appearance {
            chip: Chip::Bcm2712,
            index,
            serial: Some("e5226d57".into()),
            transport: Box::new(Wire {
                replies: replies.into(),
                sent: Arc::new(Mutex::new(Vec::new())),
            }),
        }
    }

    /// A port that hands out scripted appearances in order. `disk` marks a
    /// disk on the port: the port's view has no way to reach it.
    struct Fake {
        appearances: VecDeque<Result<Option<Appearance>, UsbBootError>>,
        #[allow(dead_code)]
        disk: bool,
    }

    impl Port for Fake {
        async fn appearance(
            &mut self,
            _previous: Option<u8>,
            _wait: Duration,
            _cancel: &AtomicBool,
        ) -> Result<Option<Appearance>, UsbBootError> {
            self.appearances.pop_front().unwrap_or(Ok(None))
        }
    }

    /// A boot-files directory with the second stage for BCM2712 and a
    /// `config.txt` at the top.
    fn boot_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("boot-only-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("2712")).unwrap();
        std::fs::write(dir.join("2712/bootcode5.bin"), [1u8; 64]).unwrap();
        std::fs::write(dir.join("config.txt"), b"boot_ramdisk=1").unwrap();
        dir
    }

    fn sha256(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    async fn job(port: &mut Fake, files: &BootFiles, timeout: Duration) -> Vec<Event> {
        let events = Mutex::new(Vec::new());
        let cancel = AtomicBool::new(false);
        let end = run(port, files, timeout, &cancel, &|event| {
            events.lock().unwrap().push(event)
        })
        .await;
        let mut events = events.into_inner().unwrap();
        events.push(end);
        events
    }

    /// The Pi's boot: the boot ROM round, then the file server round.
    fn one_boot(boot_rom_again: bool) -> VecDeque<Result<Option<Appearance>, UsbBootError>> {
        let mut appearances = VecDeque::from([
            Ok(Some(appearance(0, vec![second_stage_ok()]))),
            Ok(Some(appearance(
                1,
                vec![
                    request(0, "config.txt"),
                    request(1, "config.txt"),
                    request(0, "cmdline.txt"),
                    request(2, ""),
                ],
            ))),
        ]);
        if boot_rom_again {
            appearances.push_back(Ok(Some(appearance(0, vec![second_stage_ok()]))));
        }
        appearances
    }

    #[test]
    fn the_module_names_no_block_device_or_claim() {
        let source = include_str!("boot_only.rs");
        for needle in [concat!("atlas", "_blockdev"), concat!("claim", "::")] {
            assert!(!source.contains(needle), "boot_only.rs names {needle}");
        }
    }

    #[test]
    fn ports_map_to_nusb_locations() {
        assert_eq!(UsbPort::at("3-1.1").unwrap().location, "003-1.1");
        assert_eq!(UsbPort::at("12-2").unwrap().location, "012-2");
        assert!(UsbPort::at("x-2").is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn one_full_boot_is_served_and_each_file_reported() {
        let dir = boot_dir("served");
        let files = BootFiles::open(&dir).unwrap();
        let mut port = Fake {
            appearances: one_boot(false),
            disk: false,
        };

        let events = job(&mut port, &files, Duration::from_secs(5)).await;

        assert!(events.contains(&Event::UsbStep {
            step: UsbStep::SecondStage {
                chip: Chip::Bcm2712.label().into(),
                bytes: 64,
            },
        }));
        assert!(events.contains(&Event::BootFile {
            name: "config.txt".into(),
            served: true,
            bytes: Some(14),
            sha256: Some(sha256(b"boot_ramdisk=1")),
        }));
        assert!(events.contains(&Event::BootFile {
            name: "cmdline.txt".into(),
            served: false,
            bytes: None,
            sha256: None,
        }));
        assert_eq!(
            events.last(),
            Some(&Event::BootEnd {
                outcome: BootOutcome::Done,
                message: None,
            })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn a_boot_rom_after_the_loop_is_left_alone_and_reported() {
        let dir = boot_dir("one-shot");
        let files = BootFiles::open(&dir).unwrap();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut appearances = one_boot(false);
        appearances.push_back(Ok(Some(Appearance {
            chip: Chip::Bcm2712,
            index: 0,
            serial: Some("e5226d57".into()),
            transport: Box::new(Wire {
                replies: VecDeque::from([second_stage_ok()]),
                sent: sent.clone(),
            }),
        })));
        let mut port = Fake {
            appearances,
            disk: false,
        };

        let events = job(&mut port, &files, Duration::from_secs(5)).await;

        assert!(events.contains(&Event::UsbStep {
            step: UsbStep::NotServed {
                reason: "one-shot".into(),
                serial: Some("e5226d57".into()),
            },
        }));
        // Left alone: no second stage was sent to it.
        assert!(sent.lock().unwrap().is_empty());
        let second_stages = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::UsbStep {
                        step: UsbStep::SecondStage { .. }
                    }
                )
            })
            .count();
        assert_eq!(second_stages, 1);
        assert!(matches!(
            events.last(),
            Some(Event::BootEnd {
                outcome: BootOutcome::Done,
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn a_disk_on_the_port_changes_nothing_the_job_does() {
        let dir = boot_dir("disk");
        let files = BootFiles::open(&dir).unwrap();
        let mut without = Fake {
            appearances: one_boot(false),
            disk: false,
        };
        let mut with = Fake {
            appearances: one_boot(false),
            disk: true,
        };

        let plain = job(&mut without, &files, Duration::from_secs(5)).await;
        let disked = job(&mut with, &files, Duration::from_secs(5)).await;

        assert_eq!(plain, disked);
        assert!(
            disked
                .iter()
                .all(|event| !format!("{event:?}").contains("Disk"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn no_pi_ends_the_job_at_the_timeout() {
        let dir = boot_dir("timeout");
        let files = BootFiles::open(&dir).unwrap();
        let mut port = Fake {
            appearances: VecDeque::new(),
            disk: false,
        };

        let events = job(&mut port, &files, Duration::from_secs(3)).await;

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events.last(),
            Some(Event::BootEnd {
                outcome: BootOutcome::Timeout,
                message: Some(_),
            })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_ends_the_job_as_cancelled() {
        let dir = boot_dir("cancel");
        let files = BootFiles::open(&dir).unwrap();
        let mut port = Fake {
            appearances: VecDeque::from([Err(UsbBootError::Cancelled)]),
            disk: false,
        };

        let events = job(&mut port, &files, Duration::from_secs(3)).await;

        assert!(matches!(
            events.last(),
            Some(Event::BootEnd {
                outcome: BootOutcome::Cancelled,
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
