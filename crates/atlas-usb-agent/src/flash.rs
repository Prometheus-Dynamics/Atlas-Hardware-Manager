//! A flash on this host, as Atlas's own recovery does it on its own: USB
//! boot the board at a port with Raspberry Pi's mass-storage gadget, wait
//! for its eMMC to show as a disk on that port, write and read back the
//! image, put files on its boot partition, and restart it through the
//! gadget's console. It writes only a disk on that port whose USB device is
//! the Pi gadget, and reports each step to a [`Sink`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_blockdev::{Disk, DiskTarget, ResumePolicy, WriteOptions, WriteProgress};
use atlas_usb_agent::{Event, UsbDevice, UsbKind};
use atlas_usb_ids::{BROADCOM_VENDOR_ID, STORAGE_GADGET_PRODUCT_ID, hex};
use atlas_usbboot::{BootEvent, BootFiles, BootOptions};

use crate::claim::{self, Claim};

/// How long the eMMC may take to show as a disk after USB boot.
const DISK_TIMEOUT: Duration = Duration::from_secs(60);
const DISK_POLL: Duration = Duration::from_millis(500);
/// How long the board may take to come back after its link drops: a hub
/// reset, or a fall back to its ROM and a new USB boot.
const REATTACH_TIMEOUT: Duration = Duration::from_secs(180);
const REATTACH_POLL_MAX: Duration = Duration::from_secs(4);
const USB_DEVICES: &str = "/sys/bus/usb/devices";

/// Where a flash's events go.
pub(crate) type Sink = Arc<dyn Fn(Event) + Send + Sync>;

pub(crate) fn log(sink: &Sink, text: impl Into<String>) {
    sink(Event::Log {
        text: text.into(),
        rss_kib: rss_kib(),
    });
}

/// `VmRSS` from /proc/self/status.
pub(crate) fn rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()
}

pub(crate) struct Flash {
    pub port: String,
    pub boot_files: PathBuf,
    pub image: PathBuf,
    pub sha256: Option<String>,
    /// Path on the boot partition, and its contents.
    pub boot_writes: BTreeMap<String, String>,
}

/// The sysfs port in nusb's location form: `3-1.1` is `003-1.1`.
fn nusb_location(port: &str) -> Option<String> {
    let (bus, ports) = port.split_once('-')?;
    Some(format!("{:03}-{ports}", bus.parse::<u16>().ok()?))
}

fn read_attr(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The USB ids at `port`, from sysfs.
fn usb_ids(port: &str) -> Option<(String, String)> {
    let dir = Path::new(USB_DEVICES).join(port);
    Some((
        read_attr(&dir, "idVendor")?.to_ascii_lowercase(),
        read_attr(&dir, "idProduct")?.to_ascii_lowercase(),
    ))
}

/// Every USB device on this host, from sysfs.
pub(crate) fn usb_devices() -> Vec<UsbDevice> {
    let mut found: Vec<UsbDevice> = std::fs::read_dir(USB_DEVICES)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let port = entry.file_name().to_string_lossy().into_owned();
            // Devices only (`3-1.2`), not interfaces or root hubs.
            if !atlas_usb_agent::valid_port(&port) {
                return None;
            }
            let dir = entry.path();
            let vendor = read_attr(&dir, "idVendor")?.to_ascii_lowercase();
            let product = read_attr(&dir, "idProduct")?.to_ascii_lowercase();
            Some(UsbDevice {
                kind: UsbDevice::kind_of(&vendor, &product),
                port,
                vendor,
                product,
                name: read_attr(&dir, "product"),
                serial: read_attr(&dir, "serial"),
            })
        })
        .collect();
    found.sort_by(|a, b| a.port.cmp(&b.port));
    found
}

/// Raspberry Pi's mass-storage gadget, the only USB device written.
fn is_gadget(port: &str) -> bool {
    usb_ids(port).is_some_and(|(vendor, product)| {
        vendor == hex(BROADCOM_VENDOR_ID) && product == hex(STORAGE_GADGET_PRODUCT_ID)
    })
}

/// The Pi gadget's disk on `port`, when it shows.
fn disk_on(port: &str) -> Result<Option<Disk>, String> {
    if !is_gadget(port) {
        return Ok(None);
    }
    let disks = atlas_blockdev::list_disks().map_err(|error| error.to_string())?;
    Ok(disks
        .into_iter()
        .find(|disk| disk.usb && disk.size_bytes > 0 && disk.usb_port.as_deref() == Some(port)))
}

/// This host's own board serial (a SystemCore is a CM5), from its device
/// tree.
fn host_serial() -> Option<String> {
    std::fs::read_to_string("/proc/device-tree/serial-number")
        .ok()
        .map(|text| text.trim_matches(['\0', ' ', '\n']).to_ascii_lowercase())
        .filter(|serial| !serial.is_empty())
}

/// Whether two board serials name the same chip: Raspberry Pi shows the
/// last eight hex digits in some places and all sixteen in others.
fn same_board(a: &str, b: &str) -> bool {
    let tail = |serial: &str| {
        let serial = serial.to_ascii_lowercase();
        serial[serial.len().saturating_sub(8)..].to_string()
    };
    a.len() >= 8 && b.len() >= 8 && tail(a) == tail(b)
}

/// What may be USB booted: a device on one of this host's USB ports (this
/// host's own gadget side is never listed there), in a Raspberry Pi boot
/// ROM, and not this host's own chip.
fn check_boot_target(
    port: &str,
    ids: Option<(String, String)>,
    serial: Option<&str>,
    host: Option<&str>,
) -> Result<(), String> {
    if !atlas_usb_agent::valid_port(port) {
        return Err(format!("refusing to USB boot {port:?}: not a USB port"));
    }
    let Some((vendor, product)) = ids else {
        return Err(format!("refusing to USB boot USB {port}: nothing is there"));
    };
    if UsbDevice::kind_of(&vendor, &product) != UsbKind::UsbBoot {
        return Err(format!(
            "refusing to USB boot USB {port}: it is {vendor}:{product}, not a Raspberry Pi boot ROM"
        ));
    }
    if let (Some(serial), Some(host)) = (serial, host)
        && same_board(serial, host)
    {
        return Err(format!(
            "refusing to USB boot USB {port}: its serial {serial} is this host's own"
        ));
    }
    Ok(())
}

async fn boot(
    boot_files: &Path,
    port: &str,
    sink: &Sink,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let dir = Path::new(USB_DEVICES).join(port);
    let serial = read_attr(&dir, "serial");
    check_boot_target(
        port,
        usb_ids(port),
        serial.as_deref(),
        host_serial().as_deref(),
    )?;
    log(
        sink,
        format!(
            "USB booting the boot ROM on USB {port} (serial {})",
            serial.as_deref().unwrap_or("none")
        ),
    );
    let files = BootFiles::open(boot_files).map_err(|error| error.to_string())?;
    let location = nusb_location(port).ok_or("bad port")?;
    let options = BootOptions {
        location: Some(location),
        ..BootOptions::default()
    };
    let stop = cancel.clone();
    let events = sink.clone();
    let booted = atlas_usbboot::boot_device(
        &files,
        &options,
        &move |event| match event {
            BootEvent::SecondStage { chip, bytes } => {
                log(
                    &events,
                    format!("sent second stage for {} ({bytes} bytes)", chip.label()),
                );
                events(Event::Boot { progress: 0.3 });
            }
            BootEvent::FileSent { name, bytes } => {
                log(&events, format!("sent {name} ({bytes} bytes)"));
                if name == "boot.img" {
                    events(Event::Boot { progress: 0.9 });
                }
            }
            BootEvent::FileServerDone => events(Event::Boot { progress: 1.0 }),
            _ => {}
        },
        &move || stop.load(Ordering::Relaxed),
    )
    .await
    .map_err(|error| match error.fix() {
        Some(fix) => format!("{error}. {fix}"),
        None => error.to_string(),
    })?;
    sink(Event::Booted {
        serial: booted.serial,
        metadata: booted.metadata,
    });
    Ok(())
}

/// Runs a flash to its end: `done`, or the reason it stopped.
pub(crate) async fn flash(
    flash: Flash,
    sink: Sink,
    cancel: Arc<AtomicBool>,
) -> Result<Event, String> {
    if !flash.image.is_file() {
        return Err(format!("{}: no such image", flash.image.display()));
    }
    // Claimed before the disk can show: the automounter acts on its arrival.
    let _claim = Claim::install(
        read_attr(&Path::new(USB_DEVICES).join(&flash.port), "serial").as_deref(),
        &sink,
    );
    let disk = match disk_on(&flash.port)? {
        Some(disk) => {
            log(
                &sink,
                format!(
                    "the eMMC already shows as {} on USB {}; skipping USB boot",
                    disk.name, flash.port
                ),
            );
            disk
        }
        None => {
            match usb_ids(&flash.port) {
                Some((vendor, product))
                    if vendor == hex(BROADCOM_VENDOR_ID)
                        && product != hex(STORAGE_GADGET_PRODUCT_ID) => {}
                Some((vendor, product)) => {
                    return Err(format!(
                        "USB {} is {vendor}:{product}, not a Raspberry Pi in USB boot",
                        flash.port
                    ));
                }
                None => return Err(format!("nothing is on USB {}", flash.port)),
            }
            boot(&flash.boot_files, &flash.port, &sink, &cancel).await?;
            log(&sink, "waiting for the eMMC to show as a disk");
            let deadline = Instant::now() + DISK_TIMEOUT;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                if let Some(disk) = disk_on(&flash.port)? {
                    break disk;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "the board booted but its eMMC never showed as a disk on USB {} within {} s",
                        flash.port,
                        DISK_TIMEOUT.as_secs()
                    ));
                }
                tokio::time::sleep(DISK_POLL).await;
            }
        }
    };
    atlas_blockdev::check_target(&disk, None).map_err(|error| error.to_string())?;
    sink(Event::Disk {
        name: disk.name.clone(),
        size_bytes: disk.size_bytes,
    });
    // Nothing of a fresh disk may be mounted or open while it is written.
    // Off the runtime: it runs udevadm and scans /proc, and the agent's HTTP
    // API shares the runtime.
    let (taken, claimed) = (disk.clone(), sink.clone());
    tokio::task::spawn_blocking(move || claim::take(&taken, &claimed))
        .await
        .map_err(|error| error.to_string())??;

    let options = WriteOptions {
        verify: true,
        image_sha256: flash.sha256.clone(),
    };
    // The gadget's USB serial carries the board's: how it is found again.
    let serial = read_attr(&Path::new(USB_DEVICES).join(&flash.port), "serial");
    let journal = atlas_blockdev::journal_path(&flash.image);
    let journal_id = serial.clone().unwrap_or_else(|| flash.port.clone());
    let mut reattach = Reattach {
        boot_files: flash.boot_files.clone(),
        port: flash.port.clone(),
        serial,
        runtime: tokio::runtime::Handle::current(),
        sink: sink.clone(),
        cancel: cancel.clone(),
    };
    let target = disk.clone();
    let image = flash.image.clone();
    let stop = cancel.clone();
    let events = sink.clone();
    let (report, disk) = tokio::task::spawn_blocking(move || {
        let mut target =
            DiskTarget::open(target, Box::new(move |deadline| reattach.find(deadline)))
                .map_err(|error| error.to_string())?;
        let mut last = Instant::now() - Duration::from_secs(1);
        let written = atlas_blockdev::write_resumable(
            &image,
            &mut target,
            &options,
            &ResumePolicy::default(),
            Some((&journal, &journal_id)),
            &mut |progress| {
                // A line a quarter second at most, and every phase change.
                let phase_only = !matches!(
                    progress,
                    WriteProgress::Writing { .. }
                        | WriteProgress::Verifying { .. }
                        | WriteProgress::CheckingImage { .. }
                );
                if phase_only || last.elapsed() >= Duration::from_millis(250) {
                    last = Instant::now();
                    events(Event::Write(progress));
                }
            },
            &stop,
        )
        .map_err(|error| error.to_string())?;
        Ok::<_, String>((written, target.disk().clone()))
    })
    .await
    .map_err(|error| error.to_string())??;
    sink(Event::Written {
        bytes: report.bytes_written,
        seconds: report.seconds,
        sha256: report.sha256,
    });

    for (path, contents) in &flash.boot_writes {
        match atlas_blockdev::write_boot_file(&disk, path, contents) {
            Ok(()) => log(&sink, format!("wrote {path} on the boot partition")),
            Err(error) => log(&sink, format!("could not write {path}: {error}")),
        }
    }

    log(
        &sink,
        "restarting the board: it first puts what it still holds on the eMMC, which can take a few minutes",
    );
    let _ = atlas_blockdev::unmount_all(&disk);
    let name = disk.name.clone();
    let restarted = tokio::task::spawn_blocking(move || atlas_blockdev::restart_gadget(&name))
        .await
        .map_err(|error| error.to_string())?;
    Ok(match restarted {
        Ok(tty) => Event::Done {
            restarted: Some(tty),
            restart_error: None,
        },
        Err(reason) => Event::Done {
            restarted: None,
            restart_error: Some(reason),
        },
    })
}

/// Finds the board again after its link drops.
struct Reattach {
    boot_files: PathBuf,
    port: String,
    serial: Option<String>,
    runtime: tokio::runtime::Handle,
    sink: Sink,
    cancel: Arc<AtomicBool>,
}

impl Reattach {
    /// Waits, backing off, for the board's gadget to show again (by serial,
    /// on any port) with its disk; a board back in its ROM on its port gets
    /// the bootloader again.
    fn find(&mut self, deadline: Instant) -> Result<Disk, String> {
        let deadline = deadline.min(Instant::now() + REATTACH_TIMEOUT);
        let mut wait = Duration::from_millis(250);
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let devices = usb_devices();
            let gadget = devices.iter().find(|device| {
                device.kind == UsbKind::Gadget
                    && match &self.serial {
                        Some(serial) => device.serial.as_ref() == Some(serial),
                        None => device.port == self.port,
                    }
            });
            if let Some(gadget) = gadget {
                if let Some(disk) = disk_on(&gadget.port)? {
                    atlas_blockdev::check_target(&disk, None).map_err(|error| error.to_string())?;
                    claim::take(&disk, &self.sink)?;
                    if gadget.port != self.port {
                        log(
                            &self.sink,
                            format!("the board moved to USB {}", gadget.port),
                        );
                        self.port.clone_from(&gadget.port);
                    }
                    log(
                        &self.sink,
                        format!("the board is back: its eMMC shows as {}", disk.name),
                    );
                    return Ok(disk);
                }
            } else if devices
                .iter()
                .any(|device| device.port == self.port && device.kind == UsbKind::UsbBoot)
            {
                log(
                    &self.sink,
                    "the board fell back to its boot ROM; sending the bootloader again",
                );
                self.runtime.block_on(boot(
                    &self.boot_files,
                    &self.port,
                    &self.sink,
                    &self.cancel,
                ))?;
                wait = Duration::from_millis(250);
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "the board did not show again on USB within {} s",
                    REATTACH_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(wait.min(deadline - now));
            wait = (wait * 2).min(REATTACH_POLL_MAX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_boot_rom_that_isnt_this_host_is_usb_booted() {
        let rom = || Some(("0a5c".to_string(), "2712".to_string()));
        assert!(
            check_boot_target(
                "3-1.1.4.1",
                rom(),
                Some("e5226d57"),
                Some("10000000880f3c99")
            )
            .is_ok()
        );
        assert!(check_boot_target("3-1.1.4.1", rom(), None, None).is_ok());
        // The host's own chip, whichever length either serial is shown at.
        let own = check_boot_target("1-4", rom(), Some("880f3c99"), Some("10000000880f3c99"));
        assert!(own.unwrap_err().contains("this host's own"));
        assert!(
            check_boot_target("1-4", rom(), Some("10000000880F3C99"), Some("880f3c99")).is_err()
        );
        // Not a ROM: a gadget, a running board, or nothing.
        let gadget = Some(("0a5c".to_string(), "0104".to_string()));
        assert!(check_boot_target("3-1.1.4.1", gadget, None, None).is_err());
        assert!(check_boot_target("3-1.1.4.1", None, None, None).is_err());
        assert!(check_boot_target("usb0", rom(), None, None).is_err());
    }

    #[test]
    fn ports_map_to_nusb_locations() {
        assert_eq!(nusb_location("3-1.1").as_deref(), Some("003-1.1"));
        assert_eq!(nusb_location("12-2").as_deref(), Some("012-2"));
        assert_eq!(nusb_location("x-2"), None);
    }
}
