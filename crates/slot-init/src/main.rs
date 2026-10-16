//! `slot-init`: the slot initramfs's /init (as `/init`, PID 1: boot.rs),
//! and the slots writer's helper in the OS:
//!
//!   slot-init check-initramfs <file>          whether the kernel can unpack it (its format; xz:
//!                                             a dictionary of at most 1 MiB)
//!   slot-init boot                            /init's steps with recording ops, for
//!                                             tests/slot-init.sh: SLOT_ROOT (the prefix),
//!                                             SLOT_CALLS (the record), SLOT_FAIL (steps
//!                                             that fail), SLOT_SYS, SLOT_DEV
//!   slot-init arg <key>                       a kernel command line value (atlas.slot)
//!   slot-init find-partition PARTUUID=<id>|LABEL=<name>
//!                                             the partition's /dev node (MBR/GPT ids and ext labels
//!                                             read here; BusyBox's findfs has no PARTUUID). Test
//!                                             hooks: SLOT_SYS, SLOT_DEV (sysfs and /dev)
//!   slot-init kernel <slot.json>              the kernel release the slot was built for
//!   slot-init family <slot.json>              its OS family
//!   slot-init data-dirs <slot.json>           its per-family data dirs, one a line
//!   slot-init previous <state.json>           the slot to fall back to (none: exit 1)
//!   slot-init reboot-tryboot                  restart once into tryboot.txt's slot
//!   slot-init field <file.json> <a.b.c>       one field (missing: exit 1)
//!   slot-init boot-files <slot.json>          "<sha256>  <name>" per boot file (sha256sum -c's format)
//!   slot-init open <image> <slot.json> <root hash> [name]
//!                                             a read-only loop + dm-verity device; prints /dev/dm-N
//!
//! Exit status: 0 done, 1 refused or failed (the reason on stderr), 2 usage.

// It reports on stdout and stderr: that's its interface with /init.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use slot_init::{Manifest, State, boot_files, cmdline_arg, field, root_hash, valid_id};

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
        ["check-initramfs", file] => {
            use std::io::Read;
            let mut head = vec![0u8; 4096];
            let n = std::fs::File::open(file)
                .and_then(|mut f| f.read(&mut head))
                .map_err(|e| format!("{file}: {e}"))?;
            head.truncate(n);
            slot_init::initramfs::check(&head).map_err(|why| format!("{file}: {why}"))
        }
        ["find-partition", spec] => {
            let sys = std::env::var("SLOT_SYS").unwrap_or_else(|_| "/sys".into());
            let dev = std::env::var("SLOT_DEV").unwrap_or_else(|_| "/dev".into());
            slot_init::partition::find(
                &slot_init::partition::Spec::parse(spec)?,
                std::path::Path::new(&sys),
                std::path::Path::new(&dev),
            )
            .map(|path| path.display().to_string())
        }
        ["kernel", manifest] => Ok(Manifest::parse(&read(manifest)?)?.kernel),
        ["family", manifest] => Ok(Manifest::parse(&read(manifest)?)?.family),
        ["data-dirs", manifest] => Ok(Manifest::parse(&read(manifest)?)?.data_dirs.join("\n")),
        ["previous", state] => State::parse(&read(state)?)?
            .previous
            .filter(|id| valid_id(id))
            .ok_or_else(|| "no previous slot".to_owned()),
        ["reboot-tryboot"] => reboot().map(|()| String::new()),
        ["field", file, path] => {
            field(&read(file)?, path)?.ok_or_else(|| format!("no {path} in {file}"))
        }
        ["boot-files", manifest] => Ok(boot_files(&read(manifest)?)?
            .into_iter()
            .map(|(sha, name)| format!("{sha}  {name}"))
            .collect::<Vec<_>>()
            .join("\n")),
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

/// /init's steps under a prefix, recorded: for tests/slot-init.sh.
fn boot_recorded() -> String {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let root = std::path::PathBuf::from(var("SLOT_ROOT"));
    let env = slot_init::boot::Env {
        sys: std::path::PathBuf::from(var("SLOT_SYS")),
        dev: std::path::PathBuf::from(var("SLOT_DEV")),
        root,
        wait: std::time::Duration::from_millis(300),
        rescue_wait: std::time::Duration::ZERO,
    };
    let ops = slot_init::record::Record {
        calls: var("SLOT_CALLS").into(),
        fail: var("SLOT_FAIL").split(',').map(str::to_owned).collect(),
    };
    slot_init::boot::boot(&ops, &env)
}

/// As the initramfs's /init: boot the slot.
#[cfg(target_os = "linux")]
fn init() -> ExitCode {
    slot_init::sys::early();
    let env = slot_init::boot::Env {
        root: "/".into(),
        sys: "/sys".into(),
        dev: "/dev".into(),
        wait: std::time::Duration::from_secs(10),
        rescue_wait: std::time::Duration::from_secs(30),
    };
    let why = slot_init::boot::boot(&slot_init::sys::System, &env);
    // Every restart failed: PID 1 exiting panics the kernel, and panic=5
    // restarts it.
    eprintln!("slot-init: {why}");
    ExitCode::from(1)
}

#[cfg(not(target_os = "linux"))]
fn init() -> ExitCode {
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let argv0 = std::env::args().next().unwrap_or_default();
    if argv0.rsplit('/').next() == Some("init") {
        return init();
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["boot"] {
        let why = boot_recorded();
        if why.is_empty() {
            return ExitCode::SUCCESS;
        }
        eprintln!("slot-init: {why}");
        return ExitCode::from(1);
    }
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
