//! `slot-init`, for the slot initramfs's /init:
//!
//!   slot-init arg <key>                       a kernel command line value (atlas.slot)
//!   slot-init kernel <slot.json>              the kernel release the slot was built for
//!   slot-init family <slot.json>              its OS family
//!   slot-init data-dirs <slot.json>           its per-family data dirs, one a line
//!   slot-init previous <state.json>           the slot to fall back to (none: exit 1)
//!   slot-init reboot-tryboot                  restart once into tryboot.txt's slot
//!   slot-init open <image> <slot.json> <root hash> [name]
//!                                             a read-only loop + dm-verity device; prints /dev/dm-N
//!
//! Exit status: 0 done, 1 refused or failed (the reason on stderr), 2 usage.

// It reports on stdout and stderr: that's its interface with /init.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use slot_init::{Manifest, State, cmdline_arg, root_hash, valid_id};

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn run(args: &[String]) -> Result<String, String> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["arg", key] => {
            let cmdline = read("/proc/cmdline")?;
            cmdline_arg(&cmdline, key)
                .map(str::to_owned)
                .ok_or_else(|| format!("no {key}= on the kernel command line"))
        }
        ["kernel", manifest] => Ok(Manifest::parse(&read(manifest)?)?.kernel),
        ["family", manifest] => Ok(Manifest::parse(&read(manifest)?)?.family),
        ["data-dirs", manifest] => Ok(Manifest::parse(&read(manifest)?)?.data_dirs.join("\n")),
        ["previous", state] => State::parse(&read(state)?)?
            .previous
            .filter(|id| valid_id(id))
            .ok_or_else(|| "no previous slot".to_owned()),
        ["reboot-tryboot"] => reboot().map(|()| String::new()),
        ["open", image, manifest, root, rest @ ..] if rest.len() <= 1 => open(
            image,
            manifest,
            root,
            rest.first().copied().unwrap_or("slot"),
        ),
        _ => Err(String::new()),
    }
}

#[cfg(target_os = "linux")]
fn open(image: &str, manifest: &str, root: &str, name: &str) -> Result<String, String> {
    let manifest = Manifest::parse(&read(manifest)?)?;
    let root = root_hash(root)?;
    let (path, dev) = slot_init::sys::open_loop(std::path::Path::new(image))?;
    eprintln!("slot-init: {image} on {path} ({dev})");
    slot_init::sys::open_verity(name, &dev, &manifest.verity, &root)
}

#[cfg(target_os = "linux")]
fn reboot() -> Result<(), String> {
    slot_init::sys::reboot_tryboot()
}

#[cfg(not(target_os = "linux"))]
fn reboot() -> Result<(), String> {
    Err("Linux only".into())
}

#[cfg(not(target_os = "linux"))]
fn open(_: &str, _: &str, _: &str, _: &str) -> Result<String, String> {
    Err("Linux only".into())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) if e.is_empty() => {
            eprintln!(
                "usage: slot-init arg <key> | kernel <slot.json> | open <image> <slot.json> <root hash> [name]"
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("slot-init: {e}");
            ExitCode::from(1)
        }
    }
}
