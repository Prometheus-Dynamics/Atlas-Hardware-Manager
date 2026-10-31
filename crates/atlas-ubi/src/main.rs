//! atlas-ubi: UBI images and slot volumes, for a board's device package and
//! for checking an image before it's flashed.
//!
//! ```text
//! atlas-ubi info <image.ubi> [--peb-size <bytes>]
//! atlas-ubi slot-write <image|-> --size <bytes> [--volume <name> | --inactive] [--sha256 <hex>]
//! ```
//!
//! `info` prints the image's geometry and volumes. `slot-write` runs on a
//! board: it writes the image into the named volume, or into the other half
//! of the running `_a`/`_b` pair (`ubi.block=` on /proc/cmdline), then reads
//! it back. Output is one JSON object on stdout; errors are
//! `{"error": "..."}` with exit status 1.

use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage: atlas-ubi info <image> [--peb-size <bytes>] | slot-write <image|-> --size <bytes> [--volume <name> | --inactive] [--sha256 <hex>]";

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
        None => atlas_ubi::find_peb_size(&mut file).map_err(|error| error.0)?,
    };
    let image = atlas_ubi::read_image(&mut file, peb_size).map_err(|error| error.0)?;
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
        image.geometry.peb_size,
        image.geometry.vid_header_offset,
        image.geometry.data_offset,
        image.geometry.leb_size(),
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") => info(&args[1..]),
        Some("slot-write") => slot_write(&args[1..]),
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
