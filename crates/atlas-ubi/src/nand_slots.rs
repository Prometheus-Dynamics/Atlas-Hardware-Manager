//! A/B slots on a NAND board with Gaia's slot environments (Gaia
//! examples/boards/README.md#nand-slots, the Luckfox Pico Pro; its vendor
//! U-Boot is not patched). The env partition holds two 128K copies. U-Boot
//! reads the first and, when its CRC fails, the second. A slot is a whole
//! environment image: its `mtdparts` names the slot's kernel partition `boot`,
//! and its `sys_bootargs` picks the root volume and says `gaia.slot=<a|b>`,
//! with `gaia.trial=1` on a trial, whose `bootcmd` erases the first copy so any
//! later boot reads the committed second one.
//!
//! The order of the writes is the point: at every moment one valid copy
//! exists, so a power cut can't leave U-Boot without a partition table.
//! Atlas writes Gaia's prebuilt images unchanged and checks each one first.
//! Written from Gaia's contract; untested on a board.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::crc::crc32;
use crate::mtd::{Mtd, write_skipping_bad};

/// One environment copy, and the eraseblock it fills.
pub const COPY: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    pub fn letter(self) -> char {
        match self {
            Self::A => 'a',
            Self::B => 'b',
        }
    }

    /// Its UBI root volume.
    pub fn volume(self) -> String {
        format!("rootfs_{}", self.letter())
    }

    /// Its kernel partition's name while it isn't the running slot.
    pub fn boot_partition(self) -> String {
        format!("boot_{}", self.letter())
    }
}

/// What this boot is, from the kernel command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Running {
    pub slot: Slot,
    pub trial: bool,
}

fn slot_and_trial(words: &str) -> Option<Running> {
    let mut slot = None;
    let mut trial = false;
    for word in words.split_ascii_whitespace() {
        match word {
            "gaia.slot=a" => slot = Some(Slot::A),
            "gaia.slot=b" => slot = Some(Slot::B),
            "gaia.trial=1" => trial = true,
            _ => {}
        }
    }
    slot.map(|slot| Running { slot, trial })
}

/// `gaia.slot` and `gaia.trial` on a kernel command line.
pub fn running(cmdline: &str) -> Option<Running> {
    slot_and_trial(cmdline)
}

/// Whether `copy` is a whole environment copy with a valid CRC: a little-endian
/// CRC-32 of the rest, as U-Boot checks it.
pub fn copy_valid(copy: &[u8]) -> bool {
    copy.len() == COPY
        && u32::from_le_bytes([copy[0], copy[1], copy[2], copy[3]]) == crc32(&copy[4..])
}

/// The slot and trial flag a valid copy boots, from its `sys_bootargs`.
pub fn copy_boots(copy: &[u8]) -> Option<Running> {
    if !copy_valid(copy) {
        return None;
    }
    copy[4..]
        .split(|byte| *byte == 0)
        .take_while(|entry| !entry.is_empty())
        .find_map(|entry| {
            let text = std::str::from_utf8(entry).ok()?;
            slot_and_trial(text.strip_prefix("sys_bootargs=")?)
        })
}

/// Checks a prebuilt image before it goes near the flash: a valid copy that
/// boots exactly this slot, as a trial or not.
fn check_image(image: &[u8], want: Running) -> Result<(), String> {
    let name = format!(
        "env-{}{}.img",
        want.slot.letter(),
        if want.trial { "-trial" } else { "" }
    );
    match copy_boots(image) {
        Some(boots) if boots == want => Ok(()),
        Some(boots) => Err(format!(
            "this image boots slot {}{}, not what {name} should",
            boots.slot.letter(),
            if boots.trial { " as a trial" } else { "" }
        )),
        None => Err(format!(
            "this isn't {name}: not a {COPY}-byte environment with a valid CRC and gaia.slot"
        )),
    }
}

/// U-Boot skips a bad block, so raw offsets on the env partition would stop
/// matching what it reads; and a copy must fill one eraseblock.
fn check_env_partition(env: &mut dyn Mtd) -> Result<(), String> {
    if env.erase_size() as usize != COPY || env.size() < 2 * COPY as u64 {
        return Err(format!(
            "the env partition has {}-byte blocks and {} bytes; the slot environments need two {COPY}-byte blocks",
            env.erase_size(),
            env.size()
        ));
    }
    for block in [0, COPY as u64] {
        if env.is_bad(block)? {
            return Err(format!(
                "the env partition's block at {block:#x} is bad: U-Boot would skip it, so its copies aren't where Atlas would write them"
            ));
        }
    }
    Ok(())
}

fn read_copy(env: &mut dyn Mtd, index: u64) -> Result<Vec<u8>, String> {
    let mut copy = vec![0u8; COPY];
    env.read(index * COPY as u64, &mut copy)?;
    Ok(copy)
}

/// A copy as far as its validity goes: a torn page can fail its ECC instead
/// of reading back wrong, and that's an invalid copy too, not an error.
fn copy_or_invalid(env: &mut dyn Mtd, index: u64) -> Vec<u8> {
    read_copy(env, index).unwrap_or_default()
}

/// Erases a copy's block, writes `image` there and reads it back.
fn write_copy(env: &mut dyn Mtd, index: u64, image: &[u8]) -> Result<(), String> {
    let at = index * COPY as u64;
    env.erase(at)?;
    env.write(at, image)?;
    if read_copy(env, index)? != image {
        return Err(format!("the env copy at {at:#x} reads back different"));
    }
    Ok(())
}

/// What the env partition holds now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvState {
    /// What the next boot runs: the first copy when valid, else the second.
    pub next: Option<Running>,
    /// What a boot falls back to: the second copy.
    pub committed: Option<Running>,
}

pub fn env_state(env: &mut dyn Mtd) -> Result<EnvState, String> {
    let first = copy_boots(&copy_or_invalid(env, 0));
    let committed = copy_boots(&copy_or_invalid(env, 1));
    Ok(EnvState {
        next: first.or(committed),
        committed,
    })
}

/// The first half of an install: checks everything, then writes the other
/// slot's kernel. The caller writes its root volume, then [`arm_trial`].
/// Nothing the running or committed slot boots from is touched.
pub fn write_boot(
    current: Running,
    env: &mut dyn Mtd,
    boot: &mut dyn Mtd,
    trial_image: &[u8],
    kernel: &mut dyn Read,
    kernel_bytes: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<String, String> {
    preflight(current, env, trial_image)?;
    write_skipping_bad(boot, kernel, kernel_bytes, progress)
}

fn preflight(current: Running, env: &mut dyn Mtd, trial_image: &[u8]) -> Result<Slot, String> {
    if current.trial {
        return Err(format!(
            "this boot is a trial of slot {}: commit it or reboot out of it first",
            current.slot.letter()
        ));
    }
    let target = current.slot.other();
    check_image(
        trial_image,
        Running {
            slot: target,
            trial: true,
        },
    )?;
    check_env_partition(env)?;
    // The first copy is about to go; the second must be what saves the board.
    if copy_boots(&copy_or_invalid(env, 1)).is_none() {
        return Err(
            "the env partition's second copy isn't valid: a torn write would leave no environment, so nothing was changed"
                .into(),
        );
    }
    Ok(target)
}

/// The last step of an install: the trial environment in the first copy, the
/// second left alone. A torn write fails the CRC and boots the committed slot.
pub fn arm_trial(current: Running, env: &mut dyn Mtd, trial_image: &[u8]) -> Result<Slot, String> {
    let target = preflight(current, env, trial_image)?;
    write_copy(env, 0, trial_image)?;
    Ok(target)
}

/// On a trial boot: U-Boot should have erased the first copy. If it's still
/// valid its erase failed and the next boot would retry the trial, so it's
/// erased here. `Ok(true)` says that happened, to report.
pub fn trial_check(current: Running, env: &mut dyn Mtd) -> Result<bool, String> {
    if !current.trial {
        return Ok(false);
    }
    check_env_partition(env)?;
    if !copy_valid(&copy_or_invalid(env, 0)) {
        return Ok(false);
    }
    if copy_boots(&copy_or_invalid(env, 1)).is_none() {
        return Err(
            "U-Boot left the trial's copy in place and the second copy isn't valid: erasing would leave no environment; this board needs its environment rewritten".into(),
        );
    }
    env.erase(0)?;
    Ok(true)
}

/// Keeps a slot's commit image (`env-<x>.img`) in `dir` (on `/data`, which
/// both slots share), durably, for the trial boot that commits it. Checked
/// first: only an image that would commit this slot is kept.
pub fn keep_commit_image(dir: &Path, slot: Slot, image: &[u8]) -> Result<(), String> {
    check_image(image, Running { slot, trial: false })?;
    let fail = |error: std::io::Error| format!("{}: {error}", dir.display());
    std::fs::create_dir_all(dir).map_err(fail)?;
    let path = kept_path(dir, slot);
    let temporary = path.with_extension("new");
    let mut file = File::create(&temporary).map_err(fail)?;
    file.write_all(image).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    std::fs::rename(&temporary, &path).map_err(fail)?;
    // The rename lasts once the folder is synced. Windows can't open a
    // folder as a file (and the board is Linux).
    #[cfg(unix)]
    File::open(dir)
        .and_then(|folder| folder.sync_all())
        .map_err(fail)?;
    Ok(())
}

fn kept_path(dir: &Path, slot: Slot) -> PathBuf {
    dir.join(format!("env-{}.img", slot.letter()))
}

/// The commit image kept for `slot`, if it's there and still good.
pub fn kept_commit_image(dir: &Path, slot: Slot) -> Option<Vec<u8>> {
    let image = std::fs::read(kept_path(dir, slot)).ok()?;
    check_image(&image, Running { slot, trial: false }).ok()?;
    Some(image)
}

pub fn forget_commit_image(dir: &Path, slot: Slot) {
    let _ = std::fs::remove_file(kept_path(dir, slot));
}

/// Commits the slot this trial boot runs: `image` to the first copy, then only
/// once that's read back valid, to the second. Safe to repeat.
pub fn commit(current: Running, env: &mut dyn Mtd, image: &[u8]) -> Result<(), String> {
    if !current.trial {
        return Err(format!(
            "slot {} isn't on trial: nothing to commit",
            current.slot.letter()
        ));
    }
    check_image(
        image,
        Running {
            slot: current.slot,
            trial: false,
        },
    )?;
    check_env_partition(env)?;
    write_copy(env, 0, image)?;
    write_copy(env, 1, image)
}

/// Whether the second copy commits `slot`: a trial's watchdog stands down then.
pub fn committed_to(env: &mut dyn Mtd, slot: Slot) -> Result<bool, String> {
    Ok(env_state(env)?.committed == Some(Running { slot, trial: false }))
}

#[cfg(test)]
mod tests;
