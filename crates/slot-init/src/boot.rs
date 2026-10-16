//! The slot initramfs's /init (slot-init runs as it when started as PID 1,
//! or named `init`). The firmware already chose the slot (config.txt's
//! `include default.txt`, or on a "0 tryboot" restart tryboot.txt's
//! trial.txt: `os_prefix=slots/<id>/`); its cmdline.txt names it:
//!
//!   atlas.slot=<id> atlas.roothash=<sha256 hex> atlas.data=PARTUUID=<disk>-02
//!
//! This mounts /data (ext4; read-only, with a note, when its superblock
//! says it has errors: there's no e2fsck here, and the kernel replays the
//! journal itself), opens /data/slots/<id>.erofs read-only through
//! dm-verity with that root hash, mounts it, binds the OS family's data
//! dirs, and switch_roots into it.
//!
//! When it can't (no image, a kernel the image wasn't built for, verity
//! refusing): on a trial boot it restarts plainly, which boots the default
//! slot; on the default slot it restarts into the previous slot on trial
//! (trial.txt), at most 3 times in a row (board/slots/fallbacks, which the
//! writer clears on a confirmed boot); after that it waits 30 s so a
//! console shows why, and restarts. Before any restart the kernel log goes
//! to p1 (slot-init.log) and, with /data mounted, to
//! /data/board/slots/last-failure.log, so the reason survives a power cut.
//!
//! Everything it touches goes through [`Ops`]: the real one (sys.rs) makes
//! the syscalls; tests/slot-init.sh runs this with a recording one under a
//! prefix (`SLOT_ROOT`, `SLOT_CALLS`; see `main.rs`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::partition::{Spec, find};
use crate::{Manifest, State, cmdline_arg, valid_id};

const MAX_FALLBACKS: u32 = 3;
/// ext4's s_state bit for "errors detected".
const EXT4_ERROR_FS: u16 = 0x0002;

/// What /init does to the system. Each failure is a message for the log.
pub trait Ops {
    /// `mount -t <fstype> -o <opts> <source> <target>`.
    fn mount(&self, fstype: &str, opts: &str, source: &Path, target: &Path) -> Result<(), String>;
    fn bind(&self, source: &Path, target: &Path) -> Result<(), String>;
    fn move_mount(&self, source: &Path, target: &Path) -> Result<(), String>;
    fn umount(&self, target: &Path);
    /// Loads a module and what it depends on (modules.dep).
    fn load_module(&self, name: &str, release: &str) -> Result<(), String>;
    /// A read-only loop + dm-verity device over the image: its node.
    fn open_slot(&self, image: &Path, manifest: &Manifest, root: &str) -> Result<String, String>;
    fn kernel_log(&self) -> String;
    /// Writes `text` to `path` and makes it durable (the file and its
    /// directory flushed), so a restart right after keeps it.
    fn persist(&self, path: &Path, text: &str) -> Result<(), String>;
    fn sync(&self);
    /// A plain restart. Returns only when it couldn't.
    fn reboot(&self);
    /// A restart into tryboot.txt's slot. Returns only when it couldn't.
    fn reboot_tryboot(&self);
    /// Into the new root, as PID 1. On a board it returns only when it
    /// couldn't (the error).
    fn switch_root(&self, new_root: &Path) -> Result<(), String>;
    fn sleep(&self, time: Duration);
}

/// Where things are (all under `root`, `/` on a board).
pub struct Env {
    pub root: PathBuf,
    pub sys: PathBuf,
    pub dev: PathBuf,
    /// How long to wait for the data partition.
    pub wait: Duration,
    /// How long the last message stays up before a restart.
    pub rescue_wait: Duration,
}

impl Env {
    fn at(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
}

struct Boot<'a> {
    ops: &'a dyn Ops,
    env: &'a Env,
    tryboot: bool,
    slot: String,
    data_dev: Option<PathBuf>,
    /// /data is mounted where it was (not yet moved into the new root).
    data_mounted: std::cell::Cell<bool>,
}

/// Logs to the kernel log (and stderr).
pub fn log(env: &Env, message: &str) {
    use std::io::Write;
    let line = format!("slot-init: {message}\n");
    let kmsg = env.at("dev/kmsg");
    let written = std::fs::OpenOptions::new()
        .append(true)
        .open(&kmsg)
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if written.is_err() {
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
}

/// Writes `text` to `path` by way of `<path>.tmp` and a rename, each step
/// flushed (the file, then its directory): what the real ops' `persist`
/// does, and the recorded ones' too (they write the same files).
pub fn persist_file(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

impl Boot<'_> {
    fn log(&self, message: &str) {
        log(self.env, message);
    }

    /// p1 of the disk /data is on (`mmcblk0p2` -> `mmcblk0p1`).
    fn boot_dev(&self) -> PathBuf {
        match &self.data_dev {
            Some(dev) => {
                let text = dev.to_string_lossy();
                PathBuf::from(format!("{}1", text.strip_suffix('2').unwrap_or(&text)))
            }
            None => self.env.dev.join("mmcblk0p1"),
        }
    }

    /// The kernel log to p1 and /data.
    fn save_log(&self) {
        // What is kept, and where, goes to the kernel log too (and with it to
        // the console and pstore), so a save that fails is seen.
        let text = self.ops.kernel_log();
        let slots = self.env.at("data/board/slots");
        if slots.is_dir() {
            let path = slots.join("last-failure.log");
            match self.ops.persist(&path, &text) {
                Ok(()) => self.log(&format!("kept the kernel log in {}", path.display())),
                Err(error) => self.log(&format!(
                    "couldn't keep the kernel log in {}: {error}",
                    path.display()
                )),
            }
        }
        let boot = self.env.at("boot");
        match self.ops.mount("vfat", "rw", &self.boot_dev(), &boot) {
            Ok(()) => {
                let path = boot.join("slot-init.log");
                match self.ops.persist(&path, &text) {
                    Ok(()) => {
                        self.log("kept the kernel log in slot-init.log on the boot partition")
                    }
                    Err(error) => self.log(&format!(
                        "couldn't keep the kernel log on the boot partition: {error}"
                    )),
                }
                self.ops.sync();
                self.ops.umount(&boot);
            }
            Err(error) => self.log(&format!(
                "couldn't mount the boot partition ({}) for the log: {error}",
                self.boot_dev().display()
            )),
        }
        self.ops.sync();
    }

    /// Restarts (`tryboot`: into tryboot.txt's slot) with /data unmounted
    /// and everything synced first.
    fn restart(&self, tryboot: bool) {
        self.ops.sync();
        if self.data_mounted.get() {
            self.ops.umount(&self.env.at("data"));
        }
        self.ops.sync();
        if tryboot {
            self.ops.reboot_tryboot();
        } else {
            self.ops.reboot();
        }
    }

    /// No slot can start: say why, keep the log, restart.
    fn rescue(&self, why: &str) -> String {
        self.log(&format!("no slot can start: {why}"));
        self.save_log();
        self.log(&format!(
            "restarting in {} s (the reason is in slot-init.log on the boot partition)",
            self.env.rescue_wait.as_secs()
        ));
        self.ops.sleep(self.env.rescue_wait);
        self.restart(false);
        format!("no slot can start: {why}")
    }

    /// This slot didn't come up. Trial: back to the default. Default: the
    /// previous slot on trial, unless that was tried too often already.
    fn fail(&self, why: &str) -> String {
        let slot = if self.slot.is_empty() {
            "?"
        } else {
            &self.slot
        };
        self.log(&format!("slot {slot} can't start: {why}"));
        self.save_log();
        if self.tryboot {
            self.log("it was a trial boot; restarting into the default slot");
            self.restart(false);
            return why.to_owned();
        }
        let state = self.env.at("data/board/slots");
        if !state.is_dir() {
            return self.rescue(why);
        }
        let count: u32 = std::fs::read_to_string(state.join("fallbacks"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if count >= MAX_FALLBACKS {
            return self.rescue(&format!("{why}; fell back {count} times in a row"));
        }
        let previous = std::fs::read_to_string(state.join("state.json"))
            .ok()
            .and_then(|text| State::parse(&text).ok())
            .and_then(|s| s.previous)
            .filter(|id| valid_id(id));
        let Some(previous) = previous else {
            return self.rescue(&format!("{why}; no previous slot"));
        };
        if previous == self.slot {
            return self.rescue(&format!("{why}; the previous slot is this one"));
        }
        let _ = self
            .ops
            .persist(&state.join("fallbacks"), &format!("{}\n", count + 1));
        let boot = self.env.at("boot");
        if self
            .ops
            .mount("vfat", "rw", &self.boot_dev(), &boot)
            .is_ok()
        {
            let written = self.ops.persist(
                &boot.join("trial.txt"),
                &format!("os_prefix=slots/{previous}/\n"),
            );
            self.ops.sync();
            self.ops.umount(&boot);
            if written.is_ok() {
                self.log(&format!(
                    "restarting into the previous slot ({previous}) on trial"
                ));
                self.restart(true);
            }
        }
        self.rescue(&format!("{why}; couldn't write trial.txt"))
    }

    /// The data partition, by `spec`, else by the label `data`, for a while
    /// (the eMMC can take a moment to show up).
    fn find_data(&self, spec: &str) -> Result<PathBuf, String> {
        let spec = Spec::parse(spec)?;
        let label = Spec::Label("data".into());
        let tries = (self.env.wait.as_millis() / 100).max(1);
        let mut last = String::new();
        for _ in 0..tries {
            match find(&spec, &self.env.sys, &self.env.dev) {
                Ok(dev) => return Ok(dev),
                Err(error) => last = error,
            }
            if let Ok(dev) = find(&label, &self.env.sys, &self.env.dev) {
                self.log(&format!(
                    "no partition answers to the slot's data partition; using the one labelled data ({})",
                    dev.display()
                ));
                return Ok(dev);
            }
            self.ops.sleep(Duration::from_millis(100));
        }
        Err(last)
    }

    fn run(&mut self, cmdline: &str) -> String {
        let Some(slot) = cmdline_arg(cmdline, "atlas.slot") else {
            return self.rescue("no atlas.slot on the kernel command line");
        };
        self.slot = slot.to_owned();
        let Some(root) = cmdline_arg(cmdline, "atlas.roothash") else {
            return self.fail("no atlas.roothash");
        };
        let Some(data) = cmdline_arg(cmdline, "atlas.data") else {
            return self.fail("no atlas.data");
        };
        if !valid_id(slot) {
            return self.rescue(&format!("`{slot}` isn't a slot id"));
        }

        let data_dev = match self.find_data(data) {
            Ok(dev) => dev,
            Err(seen) => {
                self.log(&seen);
                return self.rescue(&format!("no data partition {data}"));
            }
        };
        self.data_dev = Some(data_dev.clone());

        // No e2fsck here: the kernel replays the journal at mount. A
        // filesystem marked as having errors is mounted read-only.
        let mut opts = "noatime,errors=remount-ro";
        if has_errors(&data_dev) {
            self.log("/data's filesystem is marked as having errors; mounting it read-only (back it up and reflash, or repair it off the board)");
            opts = "ro,noatime";
        }
        let data_dir = self.env.at("data");
        if let Err(error) = self.ops.mount("ext4", opts, &data_dev, &data_dir) {
            return self.rescue(&format!(
                "can't mount /data ({}): {error}",
                data_dev.display()
            ));
        }
        self.data_mounted.set(true);
        // A trial this far: /init ran (the writer tells "didn't reach /init"
        // from "/init stopped" by it).
        if self.tryboot {
            let _ = self.ops.persist(
                &data_dir.join("board/slots/trial-init"),
                &format!("{slot}\n"),
            );
        }

        let image = data_dir.join("slots").join(format!("{slot}.erofs"));
        let manifest_path = data_dir.join("slots").join(format!("{slot}.json"));
        let Ok(meta) = std::fs::metadata(&image) else {
            return self.fail(&format!("no image {}", image.display()));
        };
        let manifest = match std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))
            .and_then(|text| Manifest::parse(&text))
        {
            Ok(manifest) => manifest,
            Err(error) => return self.fail(&format!("unreadable manifest: {error}")),
        };
        // A cut-short image would open (verity checks blocks as they're
        // read) and fail later, in the slot's own system: refuse it here,
        // with a reason in the log.
        let size = meta.len();
        let tree_start = manifest.verity.hash_offset + u64::from(manifest.verity.hash_block_size);
        if let Some(want) = manifest.image.as_ref().map(|i| i.size)
            && want != size
        {
            return self.fail(&format!(
                "the image is {size} bytes, slot.json says {want}: damaged or cut short"
            ));
        }
        if size <= tree_start {
            return self.fail(&format!(
                "the image is {size} bytes, too short for its hash tree at {}: damaged or cut short",
                manifest.verity.hash_offset
            ));
        }
        let running = std::fs::read_to_string(self.env.at("proc/sys/kernel/osrelease"))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        if running.is_empty() {
            return self
                .fail("couldn't read the running kernel's release (/proc/sys/kernel/osrelease)");
        }
        if manifest.kernel != running {
            return self.fail(&format!(
                "built for kernel {}, this is {running}",
                manifest.kernel
            ));
        }

        for module in ["dm-mod", "dm-bufio", "dm-verity"] {
            if let Err(error) = self.ops.load_module(module, &running) {
                self.log(&format!("{module}: {error}"));
            }
        }
        let dm = match self.ops.open_slot(&image, &manifest, root) {
            Ok(dm) => dm,
            Err(error) => return self.fail(&format!("dm-verity refused it: {error}")),
        };
        let new_root = self.env.at("newroot");
        if let Err(error) = self.ops.mount("erofs", "ro", Path::new(&dm), &new_root) {
            return self.fail(&format!("can't mount the image ({dm}): {error}"));
        }

        // The OS family's own data dirs over /data (os/<family>/<dir>).
        if !manifest.family.is_empty() {
            for dir in &manifest.data_dirs {
                let from = data_dir.join("os").join(&manifest.family).join(dir);
                let to = data_dir.join(dir);
                let _ = std::fs::create_dir_all(&from);
                let _ = std::fs::create_dir_all(&to);
                if let Err(error) = self.ops.bind(&from, &to) {
                    self.log(&format!("couldn't bind {dir}: {error}"));
                }
            }
        }
        let _ = self
            .ops
            .persist(&data_dir.join("board/slots/booted"), &format!("{slot}\n"));
        // A trial that got this far: if it doesn't confirm, the next boot's
        // writer can tell its system failed, not this.
        if self.tryboot {
            let _ = self.ops.persist(
                &data_dir.join("board/slots/trial-started"),
                &format!("{slot}\n"),
            );
        }
        let moved = self.ops.move_mount(&data_dir, &new_root.join("data"));
        if moved.is_ok() {
            self.data_mounted.set(false);
        }
        if let Err(error) = moved {
            return self.fail(&format!(
                "can't move /data into the slot (no /data in the image?): {error}"
            ));
        }
        let family = if manifest.family.is_empty() {
            "no family"
        } else {
            &manifest.family
        };
        self.log(&format!(
            "starting slot {slot} ({family}, trial {})",
            u8::from(self.tryboot)
        ));
        match self.ops.switch_root(&new_root) {
            Ok(()) => String::new(),
            // /data has moved: nowhere to keep a log but p1.
            Err(error) => self.rescue(&format!("switch_root: {error}")),
        }
    }
}

/// Whether an ext2/3/4 superblock on `dev` says errors were found.
fn has_errors(dev: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let mut sb = [0u8; 64];
    let read = std::fs::File::open(dev).and_then(|mut f| {
        f.seek(SeekFrom::Start(1024))?;
        f.read_exact(&mut sb)
    });
    read.is_ok()
        && u16::from_le_bytes([sb[56], sb[57]]) == 0xef53
        && u16::from_le_bytes([sb[58], sb[59]]) & EXT4_ERROR_FS != 0
}

/// Whether the firmware booted this on trial (chosen/bootloader/tryboot is
/// a big-endian 1).
fn is_tryboot(env: &Env) -> bool {
    std::fs::read(env.at("proc/device-tree/chosen/bootloader/tryboot"))
        .is_ok_and(|b| b.len() >= 4 && b[..4] == [0, 0, 0, 1])
}

/// Boots the slot. On a board it returns only when it couldn't (the
/// reason); with recording ops, empty when it got to switch_root.
pub fn boot(ops: &dyn Ops, env: &Env) -> String {
    for dir in ["data", "newroot", "boot", "run"] {
        let _ = std::fs::create_dir_all(env.at(dir));
    }
    let cmdline = std::fs::read_to_string(env.at("proc/cmdline")).unwrap_or_default();
    let mut boot = Boot {
        ops,
        env,
        tryboot: is_tryboot(env),
        slot: String::new(),
        data_dev: None,
        data_mounted: std::cell::Cell::new(false),
    };
    boot.run(&cmdline)
}
