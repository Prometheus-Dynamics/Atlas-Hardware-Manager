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

use atlas_blockdev::{Disk, WriteOptions, WriteProgress};
use atlas_usb_agent::{Event, UsbDevice};
use atlas_usbboot::{BootEvent, BootFiles, BootOptions};

/// How long the eMMC may take to show as a disk after USB boot.
const DISK_TIMEOUT: Duration = Duration::from_secs(60);
const DISK_POLL: Duration = Duration::from_millis(500);
/// Raspberry Pi's mass-storage gadget, the only USB device written.
const GADGET_IDS: (&str, &str) = ("0a5c", "0104");
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

fn is_gadget(port: &str) -> bool {
    usb_ids(port).is_some_and(|(vendor, product)| (vendor.as_str(), product.as_str()) == GADGET_IDS)
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

/// A udev rule, for as long as it lives, that keeps the host's automounter
/// (a SystemCore mounts every USB filesystem it sees) off the Pi gadget's
/// partitions: a mount between the write and the read-back changes them.
/// Under /run, so a crash leaves nothing past the next boot.
struct NoAutomount;

const NO_AUTOMOUNT_RULE: &str = "/run/udev/rules.d/98-atlas-usb-flash.rules";

impl NoAutomount {
    fn install(sink: &Sink) -> Option<Self> {
        let rules = Path::new(NO_AUTOMOUNT_RULE).parent()?;
        std::fs::create_dir_all(rules).ok()?;
        std::fs::write(
            NO_AUTOMOUNT_RULE,
            "# atlas-usb-agent, while it flashes a board: no mounts of the Pi gadget's disk.\n\
             SUBSYSTEM==\"block\", ENV{ID_VENDOR_ID}==\"0a5c\", ENV{ID_MODEL_ID}==\"0104\", \
             ENV{ID_FS_USAGE}=\"\", ENV{UDISKS_IGNORE}=\"1\"\n",
        )
        .ok()?;
        reload_udev();
        log(
            sink,
            "kept the automounter off the board's disk while flashing",
        );
        Some(Self)
    }
}

impl Drop for NoAutomount {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(NO_AUTOMOUNT_RULE);
        reload_udev();
    }
}

fn reload_udev() {
    let _ = std::process::Command::new("udevadm")
        .args(["control", "--reload"])
        .status();
}

async fn boot(flash: &Flash, sink: &Sink, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let files = BootFiles::open(&flash.boot_files).map_err(|error| error.to_string())?;
    let location = nusb_location(&flash.port).ok_or("bad port")?;
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
                Some((vendor, product)) if vendor == "0a5c" && product != GADGET_IDS.1 => {}
                Some((vendor, product)) => {
                    return Err(format!(
                        "USB {} is {vendor}:{product}, not a Raspberry Pi in USB boot",
                        flash.port
                    ));
                }
                None => return Err(format!("nothing is on USB {}", flash.port)),
            }
            boot(&flash, &sink, &cancel).await?;
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
    let _no_automount = NoAutomount::install(&sink);
    atlas_blockdev::check_target(&disk, None).map_err(|error| error.to_string())?;
    sink(Event::Disk {
        name: disk.name.clone(),
        size_bytes: disk.size_bytes,
    });
    // Nothing of a fresh disk may be mounted while it is written.
    let _ = atlas_blockdev::unmount_all(&disk);

    let options = WriteOptions {
        verify: true,
        image_sha256: flash.sha256.clone(),
    };
    let target = disk.clone();
    let image = flash.image.clone();
    let stop = cancel.clone();
    let events = sink.clone();
    let report = tokio::task::spawn_blocking(move || {
        let mut last = Instant::now() - Duration::from_secs(1);
        atlas_blockdev::write_image(
            &image,
            &target,
            &options,
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
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_map_to_nusb_locations() {
        assert_eq!(nusb_location("3-1.1").as_deref(), Some("003-1.1"));
        assert_eq!(nusb_location("12-2").as_deref(), Some("012-2"));
        assert_eq!(nusb_location("x-2"), None);
    }
}
