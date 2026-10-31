//! atlas-usb-agent: Atlas's hands on another Linux computer that a board in
//! USB boot is plugged into, such as a SystemCore with a Raze on its hub.
//! Atlas copies it there with the boot files and the image, and runs it
//! over SSH as root (`sudo -n`):
//!
//! ```text
//! atlas-usb-agent flash --port 3-1.1 --boot-files <dir or .tar> --image <file>
//!                       [--sha256 <hex>] [--boot-file <path on p1>=<file>]...
//! atlas-usb-agent version
//! ```
//!
//! `flash` does on that computer what Atlas's own recovery does on its
//! own: USB boots the board at `--port` (sysfs form) with the boot files
//! (Raspberry Pi's mass-storage gadget), waits for its eMMC to show as a
//! disk on that port, writes and reads back the image, puts the boot files
//! on its first partition, and restarts it through the gadget's console.
//! It writes only a disk on that port whose USB device is the Pi gadget.
//!
//! Every step is a JSON line on stdout ([`Event`]); the last is `done` or
//! `error`. Closing stdin cancels (Atlas's SSH going away), except with
//! `--detached`: then it runs as its own service (`systemd-run`, with a
//! memory cap) and only stopping that service stops it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_blockdev::{Disk, WriteOptions, WriteProgress};
use atlas_usbboot::{BootEvent, BootFiles, BootOptions};
use serde::Serialize;

/// How long the eMMC may take to show as a disk after USB boot.
const DISK_TIMEOUT: Duration = Duration::from_secs(60);
const DISK_POLL: Duration = Duration::from_millis(500);
/// How often memory is reported, and where the agent stops itself.
const MEMORY_EVERY: Duration = Duration::from_secs(5);
const MEMORY_LIMIT_KIB: u64 = 1 << 20;
/// Raspberry Pi's mass-storage gadget, the only USB device written.
const GADGET_IDS: (&str, &str) = ("0a5c", "0104");

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
enum Event {
    Log {
        text: String,
        /// This process's resident memory then, in KiB.
        rss_kib: Option<u64>,
    },
    Boot {
        /// 0 to 1 through USB boot.
        progress: f32,
    },
    Booted {
        serial: Option<String>,
        metadata: std::collections::BTreeMap<String, String>,
    },
    Disk {
        name: String,
        size_bytes: u64,
    },
    Write(WriteProgress),
    Written {
        bytes: u64,
        seconds: u64,
        sha256: String,
    },
    Done {
        /// The console the restart went through, or why it didn't.
        restarted: Option<String>,
        restart_error: Option<String>,
    },
    Error {
        message: String,
    },
}

fn emit(event: &Event) {
    if let Ok(line) = serde_json::to_string(event) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// `VmRSS` from /proc/self/status.
fn rss_kib() -> Option<u64> {
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

fn log(text: impl Into<String>) {
    emit(&Event::Log {
        text: text.into(),
        rss_kib: rss_kib(),
    });
}

struct Flash {
    port: String,
    boot_files: PathBuf,
    image: PathBuf,
    sha256: Option<String>,
    /// (path on the boot partition, local file with its contents)
    boot_writes: Vec<(String, PathBuf)>,
    /// Run as its own service: stdin isn't Atlas's, so it doesn't cancel.
    detached: bool,
}

fn parse(args: &[String]) -> Result<Flash, String> {
    let mut port = None;
    let mut boot_files = None;
    let mut image = None;
    let mut sha256 = None;
    let mut boot_writes = Vec::new();
    let mut detached = false;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--port" => port = Some(value()?),
            "--boot-files" => boot_files = Some(PathBuf::from(value()?)),
            "--image" => image = Some(PathBuf::from(value()?)),
            "--sha256" => sha256 = Some(value()?),
            "--detached" => detached = true,
            "--boot-file" => {
                let pair = value()?;
                let (path, file) = pair
                    .split_once('=')
                    .ok_or_else(|| format!("--boot-file {pair}: write <path>=<file>"))?;
                boot_writes.push((path.to_string(), PathBuf::from(file)));
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let port = port.ok_or("--port is required")?;
    if !valid_port(&port) {
        return Err(format!("{port} isn't a USB port path like 3-1.1"));
    }
    Ok(Flash {
        port,
        boot_files: boot_files.ok_or("--boot-files is required")?,
        image: image.ok_or("--image is required")?,
        sha256,
        boot_writes,
        detached,
    })
}

/// `3-1.1`: a bus number, then dot-separated port numbers.
fn valid_port(port: &str) -> bool {
    port.split_once('-').is_some_and(|(bus, ports)| {
        !bus.is_empty()
            && bus.chars().all(|c| c.is_ascii_digit())
            && !ports.is_empty()
            && ports
                .split('.')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    })
}

/// The sysfs port in nusb's location form: `3-1.1` is `003-1.1`.
fn nusb_location(port: &str) -> Option<String> {
    let (bus, ports) = port.split_once('-')?;
    Some(format!("{:03}-{ports}", bus.parse::<u16>().ok()?))
}

/// The USB ids at `port`, from sysfs.
fn usb_ids(port: &str) -> Option<(String, String)> {
    let dir = Path::new("/sys/bus/usb/devices").join(port);
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|text| text.trim().to_ascii_lowercase())
    };
    Some((read("idVendor")?, read("idProduct")?))
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
    fn install() -> Option<Self> {
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
        log("kept the automounter off the board's disk while flashing");
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

async fn boot(flash: &Flash, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let files = BootFiles::open(&flash.boot_files).map_err(|error| error.to_string())?;
    let location = nusb_location(&flash.port).ok_or("bad port")?;
    let options = BootOptions {
        location: Some(location),
        ..BootOptions::default()
    };
    let stop = cancel.clone();
    let booted = atlas_usbboot::boot_device(
        &files,
        &options,
        &|event| match event {
            BootEvent::SecondStage { chip, bytes } => {
                log(format!(
                    "sent second stage for {} ({bytes} bytes)",
                    chip.label()
                ));
                emit(&Event::Boot { progress: 0.3 });
            }
            BootEvent::FileSent { name, bytes } => {
                log(format!("sent {name} ({bytes} bytes)"));
                if name == "boot.img" {
                    emit(&Event::Boot { progress: 0.9 });
                }
            }
            BootEvent::FileServerDone => emit(&Event::Boot { progress: 1.0 }),
            _ => {}
        },
        &move || stop.load(Ordering::Relaxed),
    )
    .await
    .map_err(|error| match error.fix() {
        Some(fix) => format!("{error}. {fix}"),
        None => error.to_string(),
    })?;
    emit(&Event::Booted {
        serial: booted.serial,
        metadata: booted.metadata,
    });
    Ok(())
}

async fn flash(flash: Flash, cancel: Arc<AtomicBool>) -> Result<Event, String> {
    if !flash.image.is_file() {
        return Err(format!("{}: no such image", flash.image.display()));
    }
    let disk = match disk_on(&flash.port)? {
        Some(disk) => {
            log(format!(
                "the eMMC already shows as {} on USB {}; skipping USB boot",
                disk.name, flash.port
            ));
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
            boot(&flash, &cancel).await?;
            log("waiting for the eMMC to show as a disk");
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
    let _no_automount = NoAutomount::install();
    atlas_blockdev::check_target(&disk, None).map_err(|error| error.to_string())?;
    emit(&Event::Disk {
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
                    emit(&Event::Write(progress));
                }
            },
            &stop,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    emit(&Event::Written {
        bytes: report.bytes_written,
        seconds: report.seconds,
        sha256: report.sha256,
    });

    for (path, file) in &flash.boot_writes {
        let contents = std::fs::read_to_string(file)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        match atlas_blockdev::write_boot_file(&disk, path, &contents) {
            Ok(()) => log(format!("wrote {path} on the boot partition")),
            Err(error) => log(format!("could not write {path}: {error}")),
        }
    }

    log(
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("version") => {
            log(env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("flash") => parse(&args[1..]).and_then(|options| {
            let cancel = Arc::new(AtomicBool::new(false));
            // Memory, every few seconds, and a stop well before the host's
            // own services feel it (the service cap is the hard limit).
            std::thread::spawn(|| {
                loop {
                    std::thread::sleep(MEMORY_EVERY);
                    let rss = rss_kib().unwrap_or(0);
                    if rss > MEMORY_LIMIT_KIB {
                        emit(&Event::Error {
                            message: format!(
                                "stopped: using {} MiB, over the agent's {} MiB limit",
                                rss / 1024,
                                MEMORY_LIMIT_KIB / 1024
                            ),
                        });
                        std::process::exit(3);
                    }
                    log(format!("memory {} MiB", rss / 1024));
                }
            });
            // Closing stdin (Atlas's SSH going away) cancels, unless it
            // runs detached. What comes in is read and dropped.
            if !options.detached {
                let watcher = cancel.clone();
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    let mut stdin = std::io::stdin().lock();
                    while matches!(stdin.read(&mut buffer), Ok(n) if n > 0) {}
                    watcher.store(true, Ordering::Relaxed);
                });
            }
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?
                .block_on(flash(options, cancel))
        }),
        _ => Err("usage: atlas-usb-agent flash --port <3-1.1> --boot-files <dir|tar> --image <file> [--sha256 <hex>] [--boot-file <path>=<file>]... [--detached] | version".into()),
    };
    match result {
        Ok(done) => {
            emit(&done);
            ExitCode::SUCCESS
        }
        Err(message) => {
            emit(&Event::Error { message });
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_checked_and_mapped_to_nusb_locations() {
        assert!(valid_port("3-1.1"));
        assert!(valid_port("1-4"));
        assert!(!valid_port("3-"));
        assert!(!valid_port("3-1..1"));
        assert!(!valid_port("../3-1"));
        assert!(!valid_port("usb3"));
        assert_eq!(nusb_location("3-1.1").as_deref(), Some("003-1.1"));
        assert_eq!(nusb_location("12-2").as_deref(), Some("012-2"));
    }

    #[test]
    fn options_parse() {
        let args: Vec<String> = [
            "--port",
            "3-1.1",
            "--boot-files",
            "/tmp/b.tar",
            "--image",
            "/tmp/i.img.xz",
            "--sha256",
            "ab",
            "--boot-file",
            "board/authorized_keys=/tmp/k",
        ]
        .map(str::to_string)
        .to_vec();
        let flash = parse(&args).unwrap();
        assert_eq!(flash.port, "3-1.1");
        assert_eq!(flash.sha256.as_deref(), Some("ab"));
        assert_eq!(
            flash.boot_writes,
            [("board/authorized_keys".to_string(), PathBuf::from("/tmp/k"))]
        );
        assert!(parse(&["--port".to_string(), "x".to_string()]).is_err());
        assert!(parse(&["--bogus".to_string()]).is_err());
    }
}
