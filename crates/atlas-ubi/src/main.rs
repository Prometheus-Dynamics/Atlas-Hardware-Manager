//! atlas-ubi: UBI images and slot volumes, for a board's device package and
//! for checking an image before it's flashed.
//!
//! ```text
//! atlas-ubi info <image.ubi> [--peb-size <bytes>]
//! atlas-ubi slot-write <image|-> --size <bytes> [--volume <name> | --inactive] [--sha256 <hex>]
//! atlas-ubi nand-slot status
//! atlas-ubi nand-slot install --bundle <slot-bundle.tar>
//! atlas-ubi nand-slot trial-check
//! atlas-ubi nand-slot commit [--env <env-x.img>]
//! atlas-ubi nand-slot watchdog --deadline <seconds> [--timeout <seconds>]
//! ```
//!
//! `info` prints the image's geometry and volumes. `slot-write` runs on a
//! board: it writes the image into the named volume, or into the other half
//! of the running `_a`/`_b` pair (`ubi.block=` on /proc/cmdline), then reads
//! it back.
//!
//! `nand-slot` runs on a board with Gaia's slot environments
//! (`atlas_ubi::nand_slots`). `install` takes what it needs from Gaia's bundle
//! by the target slot: it writes the other slot's kernel and root, keeps its
//! commit image on `/data` and arms its one trial; the caller reboots. On the
//! trial boot, `trial-check` runs early, and `watchdog` from then on: it resets
//! the board if the trial hangs or isn't committed by the deadline, and stands
//! down once `commit` has run. `commit` uses the kept image; if `/data` didn't
//! keep it, it asks for one with `--env` and the trial goes on. Rejecting a
//! trial is rebooting without committing.
//!
//! Output is one JSON object on stdout; errors are `{"error": "..."}` with
//! exit status 1.

use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage: atlas-ubi info <image> [--peb-size <bytes>] | slot-write <image|-> --size <bytes> [--volume <name> | --inactive] [--sha256 <hex>] | nand-slot status | nand-slot install --bundle <file> | nand-slot trial-check | nand-slot commit [--env <file>] | nand-slot watchdog --deadline <seconds> [--timeout <seconds>]";

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
}

fn info(args: &[String]) -> Result<String, String> {
    let mut path = None;
    let mut peb_size = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--peb-size" => {
                peb_size = Some(
                    args.next()
                        .and_then(|value| value.parse::<u32>().ok())
                        .ok_or("--peb-size needs a number of bytes")?,
                );
            }
            other if path.is_none() => path = Some(other.to_string()),
            other => return Err(format!("unexpected {other}")),
        }
    }
    let path = path.ok_or(USAGE)?;
    let mut file = std::fs::File::open(&path).map_err(|error| format!("{path}: {error}"))?;
    let peb_size = match peb_size {
        Some(size) => size,
        None => atlas_ubi::find_peb_size(&mut file).map_err(|error| error.to_string())?,
    };
    let image = atlas_ubi::read_image(&mut file, peb_size).map_err(|error| error.to_string())?;
    let volumes: Vec<String> = image
        .volumes
        .iter()
        .map(|volume| {
            format!(
                "{{\"id\":{},\"name\":{},\"type\":\"{}\",\"reserved_lebs\":{},\"image_lebs\":{},\"autoresize\":{}}}",
                volume.id,
                json_string(&volume.name),
                match volume.kind {
                    atlas_ubi::headers::VolumeKind::Dynamic => "dynamic",
                    atlas_ubi::headers::VolumeKind::Static => "static",
                },
                volume.reserved_lebs,
                volume.image_lebs,
                volume.autoresize
            )
        })
        .collect();
    Ok(format!(
        "{{\"peb_size\":{},\"vid_header_offset\":{},\"data_offset\":{},\"leb_size\":{},\"image_seq\":{},\"pebs\":{},\"reserved_pebs\":{},\"volumes\":[{}]}}",
        image.layout.peb_size,
        image.layout.vid_header_offset,
        image.layout.data_offset,
        image.layout.leb_size(),
        image.image_seq,
        image.pebs,
        image.reserved_pebs(),
        volumes.join(",")
    ))
}

#[cfg(target_os = "linux")]
fn slot_write(args: &[String]) -> Result<String, String> {
    use std::path::Path;
    let mut source = None;
    let mut size = None;
    let mut volume = None;
    let mut inactive = false;
    let mut sha256 = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--size" => {
                size = Some(
                    value()?
                        .parse::<u64>()
                        .map_err(|_| "--size needs a number of bytes")?,
                );
            }
            "--volume" => volume = Some(value()?),
            "--inactive" => inactive = true,
            "--sha256" => sha256 = Some(value()?),
            other if source.is_none() => source = Some(other.to_string()),
            other => return Err(format!("unexpected {other}")),
        }
    }
    let source = source.ok_or(USAGE)?;
    let size = size.ok_or("--size is required (the bytes to write)")?;
    let sysfs = Path::new("/sys/class/ubi");
    let name = match (volume, inactive) {
        (Some(name), false) => name,
        (None, true) => {
            let cmdline = std::fs::read_to_string("/proc/cmdline")
                .map_err(|error| format!("/proc/cmdline: {error}"))?;
            let running = atlas_ubi::slots::running_slot(&cmdline)
                .ok_or("this boot maps no _a/_b slot volume (no ubi.block= on the command line)")?;
            let names: Vec<String> = atlas_ubi::volume::attached_volumes(sysfs)
                .into_iter()
                .map(|volume| volume.name)
                .collect();
            atlas_ubi::slots::inactive_slot(&names, &running)?.to_string()
        }
        _ => return Err("give one of --volume <name> or --inactive".into()),
    };
    let mut reader: Box<dyn std::io::Read> = if source == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        Box::new(std::fs::File::open(&source).map_err(|error| format!("{source}: {error}"))?)
    };
    let written = atlas_ubi::volume::write_volume(
        sysfs,
        Path::new("/dev"),
        &name,
        &mut reader,
        size,
        sha256.as_deref(),
        &mut |_| {},
    )?;
    Ok(format!(
        "{{\"volume\":{},\"bytes\":{},\"sha256\":\"{}\"}}",
        json_string(&written.volume),
        written.bytes,
        written.sha256
    ))
}

#[cfg(not(target_os = "linux"))]
fn slot_write(_: &[String]) -> Result<String, String> {
    Err("slot-write runs on a Linux board".into())
}

#[cfg(target_os = "linux")]
mod nand {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use atlas_ubi::bundle::Bundle;
    use atlas_ubi::mtd::find_by_name;
    use atlas_ubi::nand_slots::{
        self, Running, arm_trial, commit, committed_to, env_state, forget_commit_image,
        keep_commit_image, kept_commit_image, trial_check, write_boot,
    };
    use atlas_ubi::sys::{LinuxMtd, Watchdog};
    use atlas_ubi::volume::write_volume;

    use super::USAGE;

    const MTD: &str = "/sys/class/mtd";
    const UBI: &str = "/sys/class/ubi";
    /// Where a trial's commit image waits: on `data`, which both slots share.
    const KEPT: &str = "/data/atlas-ubi";
    const WATCHDOG: &str = "/dev/watchdog";
    const WATCHDOG_TIMEOUT_S: u32 = 30;

    fn partition(name: &str) -> Result<LinuxMtd, String> {
        let sysfs = Path::new(MTD);
        let node = find_by_name(sysfs, name)
            .ok_or_else(|| format!("no flash partition named {name} in {MTD}"))?;
        LinuxMtd::open(sysfs, Path::new("/dev"), &node)
    }

    fn current() -> Result<Running, String> {
        let cmdline = std::fs::read_to_string("/proc/cmdline")
            .map_err(|error| format!("/proc/cmdline: {error}"))?;
        nand_slots::running(&cmdline)
            .ok_or_else(|| "this boot has no gaia.slot on its command line".to_string())
    }

    fn options(args: &[String]) -> Result<BTreeMap<String, String>, String> {
        let mut found = BTreeMap::new();
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let name = flag
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected {flag}"))?;
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            found.insert(name.to_string(), value.clone());
        }
        Ok(found)
    }

    fn required<'a>(found: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, String> {
        found
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("--{name} is required"))
    }

    fn read(path: &str) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|error| format!("{path}: {error}"))
    }

    fn running_json(running: Option<Running>) -> String {
        running.map_or_else(
            || "null".to_string(),
            |running| {
                format!(
                    "{{\"slot\":\"{}\",\"trial\":{}}}",
                    running.slot.letter(),
                    running.trial
                )
            },
        )
    }

    fn status() -> Result<String, String> {
        let running = current()?;
        let state = env_state(&mut partition("env")?)?;
        Ok(format!(
            "{{\"running\":{},\"next\":{},\"committed\":{}}}",
            running_json(Some(running)),
            running_json(state.next),
            running_json(state.committed)
        ))
    }

    fn install(args: &[String]) -> Result<String, String> {
        let found = options(args)?;
        let running = current()?;
        let target = running.slot.other();
        let letter = target.letter();
        let bundle = Bundle::open(Path::new(required(&found, "bundle")?))?;
        let trial_image = bundle.read(&format!("env-{letter}-trial.img"))?;
        let commit_image = bundle.read(&format!("env-{letter}.img"))?;
        let mut env = partition("env")?;

        // 1. The other slot's kernel, checked before anything is erased.
        let (mut kernel, kernel_bytes) = bundle.reader("boot.img")?;
        let boot_sha256 = write_boot(
            running,
            &mut env,
            &mut partition(&target.boot_partition())?,
            &trial_image,
            &mut kernel,
            kernel_bytes,
            &mut |_| {},
        )?;

        // 2. Its root: EROFS with its dm-verity tree appended, one file for
        // one volume (its env images carry the root hash), or the squashfs of
        // bundles from before.
        let (mut rootfs, rootfs_bytes) = bundle
            .reader("rootfs.erofs")
            .or_else(|_| bundle.reader("rootfs.squashfs"))
            .map_err(|_| "the bundle has neither rootfs.erofs nor rootfs.squashfs".to_string())?;
        let root = write_volume(
            Path::new(UBI),
            Path::new("/dev"),
            &target.volume(),
            &mut rootfs,
            rootfs_bytes,
            None,
            &mut |_| {},
        )?;

        // 3. What the trial boot commits with. A /data that can't keep it
        // costs only sending it again at the commit.
        let kept = keep_commit_image(Path::new(KEPT), target, &commit_image);

        // 4. The one trial; the second copy stays what it is.
        arm_trial(running, &mut env, &trial_image)?;
        Ok(format!(
            "{{\"slot\":\"{letter}\",\"boot_sha256\":\"{boot_sha256}\",\"rootfs_sha256\":\"{}\",\"commit_image_kept\":{},\"reboot\":true}}",
            root.sha256,
            kept.is_ok()
        ))
    }

    fn check() -> Result<String, String> {
        let running = current()?;
        let erased = trial_check(running, &mut partition("env")?)?;
        Ok(format!(
            "{{\"trial\":{},\"erased_stale_trial\":{erased}}}",
            running.trial
        ))
    }

    fn commit_slot(args: &[String]) -> Result<String, String> {
        let found = options(args)?;
        let running = current()?;
        let letter = running.slot.letter();
        let image = match found.get("env") {
            Some(path) => read(path)?,
            None => kept_commit_image(Path::new(KEPT), running.slot).ok_or_else(|| {
                format!(
                    "{KEPT} doesn't hold this slot's commit image; send env-{letter}.img with --env. Nothing was written, and the trial goes on"
                )
            })?,
        };
        commit(running, &mut partition("env")?, &image)?;
        forget_commit_image(Path::new(KEPT), running.slot);
        Ok(format!("{{\"committed\":\"{letter}\"}}"))
    }

    /// A trial's watchdog: fed while this runs, so a hung kernel resets the
    /// board into the committed slot; not fed past the deadline, so a trial
    /// nobody commits does too. Stands down once the commit lands.
    fn watchdog(args: &[String]) -> Result<String, String> {
        let found = options(args)?;
        let running = current()?;
        if !running.trial {
            return Ok("{\"watching\":false}".into());
        }
        let deadline: u64 = required(&found, "deadline")?
            .parse()
            .map_err(|_| "--deadline needs a number of seconds".to_string())?;
        let timeout: u32 = match found.get("timeout") {
            Some(text) => text
                .parse()
                .map_err(|_| "--timeout needs a number of seconds".to_string())?,
            None => WATCHDOG_TIMEOUT_S,
        };
        let mut env = partition("env")?;
        let mut dog = Watchdog::open(Path::new(WATCHDOG), timeout)?;
        let end = Instant::now() + Duration::from_secs(deadline);
        loop {
            if committed_to(&mut env, running.slot)? {
                dog.disarm()?;
                return Ok("{\"watching\":false,\"committed\":true}".into());
            }
            if Instant::now() >= end {
                // Not fed again: the board resets into the committed slot.
                std::thread::sleep(dog.fires_within());
                return Err(
                    "the trial's deadline passed and the watchdog didn't reset the board".into(),
                );
            }
            dog.feed()?;
            std::thread::sleep(dog.feed_every());
        }
    }

    pub fn run(args: &[String]) -> Result<String, String> {
        match args.first().map(String::as_str) {
            Some("status") => status(),
            Some("install") => install(&args[1..]),
            Some("trial-check") => check(),
            Some("commit") => commit_slot(&args[1..]),
            Some("watchdog") => watchdog(&args[1..]),
            _ => Err(USAGE.into()),
        }
    }
}

#[cfg(target_os = "linux")]
fn nand_slot(args: &[String]) -> Result<String, String> {
    nand::run(args)
}

#[cfg(not(target_os = "linux"))]
fn nand_slot(_: &[String]) -> Result<String, String> {
    Err("nand-slot runs on a Linux board".into())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") => info(&args[1..]),
        Some("slot-write") => slot_write(&args[1..]),
        Some("nand-slot") => nand_slot(&args[1..]),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(line) => {
            print(&line);
            ExitCode::SUCCESS
        }
        Err(message) => {
            print(&format!("{{\"error\":{}}}", json_string(&message)));
            ExitCode::FAILURE
        }
    }
}
