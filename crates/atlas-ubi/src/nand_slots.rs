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

use std::io::Read;

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
mod tests {
    use super::*;
    use crate::mtd::ram::RamMtd;

    /// An environment image as Gaia builds one.
    fn image(slot: Slot, trial: bool) -> Vec<u8> {
        let mut copy = vec![0xffu8; COPY];
        let trial_word = if trial { " gaia.trial=1" } else { "" };
        let text = format!(
            "mtdparts=spi-nand0:256K(env),256K@256K(idblock),512K(uboot),6M(boot),6M(boot_b),-(ubi)\0sys_bootargs=ubi.mtd=5 ubi.block=0,{} ro gaia.slot={}{trial_word}\0\0",
            slot.volume(),
            slot.letter()
        );
        copy[4..4 + text.len()].copy_from_slice(text.as_bytes());
        let crc = crc32(&copy[4..]);
        copy[..4].copy_from_slice(&crc.to_le_bytes());
        copy
    }

    /// A factory env partition: env-a twice.
    fn factory() -> RamMtd {
        let mut env = RamMtd::new(2, COPY as u32, 2048);
        let a = image(Slot::A, false);
        env.bytes[..COPY].copy_from_slice(&a);
        env.bytes[COPY..].copy_from_slice(&a);
        env
    }

    const A: Running = Running {
        slot: Slot::A,
        trial: false,
    };
    const B_TRIAL: Running = Running {
        slot: Slot::B,
        trial: true,
    };

    /// What U-Boot does on boot: reads the first copy, else the second; a
    /// trial erases the first. Returns what it boots.
    fn uboot(env: &mut RamMtd) -> Running {
        let state = env_state(env).unwrap();
        let boots = state.next.expect("U-Boot found an environment");
        if boots.trial {
            env.bytes[..COPY].fill(0xff);
        }
        boots
    }

    #[test]
    fn the_command_line_says_the_slot() {
        assert_eq!(
            running("ro gaia.slot=b gaia.trial=1 panic=5"),
            Some(B_TRIAL)
        );
        assert_eq!(running("gaia.slot=a"), Some(A));
        assert_eq!(running("root=/dev/mmcblk0p2"), None);
        assert_eq!(Slot::A.other().volume(), "rootfs_b");
        assert_eq!(Slot::B.other().boot_partition(), "boot_a");
    }

    #[test]
    fn install_trial_and_commit_in_the_contracts_order() {
        let mut env = factory();
        assert_eq!(arm_trial(A, &mut env, &image(Slot::B, true)), Ok(Slot::B));
        assert_eq!(
            env.log,
            [("erase", 0), ("write", 0)],
            "the second copy untouched"
        );
        assert_eq!(env.bytes[COPY..], image(Slot::A, false)[..]);

        // The trial boots B once; U-Boot erased its copy.
        assert_eq!(uboot(&mut env), B_TRIAL);
        assert_eq!(trial_check(B_TRIAL, &mut env), Ok(false));

        // Commit: first copy, then second, each erased first.
        env.log.clear();
        assert!(!committed_to(&mut env, Slot::B).unwrap());
        commit(B_TRIAL, &mut env, &image(Slot::B, false)).unwrap();
        assert_eq!(
            env.log,
            [
                ("erase", 0),
                ("write", 0),
                ("erase", COPY as u64),
                ("write", COPY as u64)
            ]
        );
        assert!(committed_to(&mut env, Slot::B).unwrap());
        assert_eq!(
            uboot(&mut env),
            Running {
                slot: Slot::B,
                trial: false
            }
        );
        // Repeating is harmless.
        commit(B_TRIAL, &mut env, &image(Slot::B, false)).unwrap();
    }

    #[test]
    fn a_trial_not_committed_comes_back_on_the_committed_slot() {
        let mut env = factory();
        arm_trial(A, &mut env, &image(Slot::B, true)).unwrap();
        assert_eq!(uboot(&mut env), B_TRIAL);
        // Killed in Linux, or rejected: nothing written; the next boot is A.
        assert_eq!(uboot(&mut env), A);
        assert_eq!(uboot(&mut env), A);
    }

    #[test]
    fn a_power_cut_at_any_write_leaves_a_bootable_board() {
        // Arming the trial: cut during its write (0) or the erase before it.
        let mut env = factory();
        env.power_cut_after = Some(0);
        assert!(arm_trial(A, &mut env, &image(Slot::B, true)).is_err());
        assert_eq!(uboot(&mut env), A, "a torn first copy fails its CRC");

        // Committing: cut at each of the two writes.
        for writes in 0..2 {
            let mut env = factory();
            arm_trial(A, &mut env, &image(Slot::B, true)).unwrap();
            uboot(&mut env);
            env.power_cut_after = Some(writes);
            assert!(commit(B_TRIAL, &mut env, &image(Slot::B, false)).is_err());
            env.power_cut_after = None;
            let after = uboot(&mut env);
            // Before the first write lands: still A. After: B, from copy one.
            let want = if writes == 0 { Slot::A } else { Slot::B };
            assert_eq!(after.slot, want, "cut at write {writes}");
            assert!(!after.trial);
        }
    }

    #[test]
    fn wrong_images_and_unsafe_states_are_refused_before_any_write() {
        let refused = |result: Result<Slot, String>, env: &RamMtd, word: &str| {
            let error = result.unwrap_err();
            assert!(error.contains(word), "{error}");
            assert!(env.log.is_empty(), "nothing written: {error}");
        };
        // The running slot's image, or a committed one, as the trial.
        let mut env = factory();
        let result = arm_trial(A, &mut env, &image(Slot::A, true));
        refused(result, &env, "boots slot a");
        let result = arm_trial(A, &mut env, &image(Slot::B, false));
        refused(result, &env, "env-b-trial.img");
        let result = arm_trial(A, &mut env, &[0u8; COPY]);
        refused(result, &env, "valid CRC");
        // On a trial boot: commit or leave first.
        let result = arm_trial(B_TRIAL, &mut env, &image(Slot::A, true));
        refused(result, &env, "trial");
        // A bad env block.
        let mut bad = factory();
        bad.bad.push(COPY as u64);
        let result = arm_trial(A, &mut bad, &image(Slot::B, true));
        refused(result, &bad, "bad");
        // No valid second copy: a torn write would leave nothing.
        let mut torn = factory();
        torn.bytes[COPY + 9] ^= 1;
        let result = arm_trial(A, &mut torn, &image(Slot::B, true));
        refused(result, &torn, "second copy");
        // Another geometry.
        let mut other = RamMtd::new(4, 64 * 1024, 2048);
        let result = arm_trial(A, &mut other, &image(Slot::B, true));
        refused(result, &other, "two 131072-byte blocks");
        // Commit: only on a trial, only its own slot's committed image.
        let mut env = factory();
        assert!(
            commit(A, &mut env, &image(Slot::A, false))
                .unwrap_err()
                .contains("isn't on trial")
        );
        assert!(commit(B_TRIAL, &mut env, &image(Slot::B, true)).is_err());
        assert!(commit(B_TRIAL, &mut env, &image(Slot::A, false)).is_err());
        assert!(env.log.is_empty());
    }

    /// A flash whose first copy can't be read at all (a torn page's ECC).
    struct EccFails(RamMtd);

    impl Mtd for EccFails {
        fn size(&self) -> u64 {
            self.0.size()
        }
        fn erase_size(&self) -> u32 {
            self.0.erase_size()
        }
        fn page_size(&self) -> u32 {
            self.0.page_size()
        }
        fn is_bad(&mut self, offset: u64) -> Result<bool, String> {
            self.0.is_bad(offset)
        }
        fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
            if offset == 0 {
                return Err("Bad message".into());
            }
            self.0.read(offset, buffer)
        }
        fn erase(&mut self, offset: u64) -> Result<(), String> {
            self.0.erase(offset)
        }
        fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), String> {
            self.0.write(offset, data)
        }
    }

    #[test]
    fn an_unreadable_first_copy_is_an_invalid_one() {
        let mut env = EccFails(factory());
        let state = env_state(&mut env).unwrap();
        assert_eq!(
            state.next,
            Some(A),
            "falls to the second copy, as U-Boot does"
        );
        assert_eq!(trial_check(B_TRIAL, &mut env), Ok(false));
    }

    #[test]
    fn a_failed_uboot_erase_is_caught_on_the_trial_boot() {
        let mut env = factory();
        arm_trial(A, &mut env, &image(Slot::B, true)).unwrap();
        // U-Boot booted the trial but its erase failed (a wrong mtd name).
        assert_eq!(trial_check(B_TRIAL, &mut env), Ok(true));
        assert!(!copy_valid(&env.bytes[..COPY]));
        assert_eq!(uboot(&mut env), A, "the next boot is the committed slot");
        // With no valid second copy it refuses rather than leave none.
        let mut env = factory();
        arm_trial(A, &mut env, &image(Slot::B, true)).unwrap();
        env.bytes[COPY + 9] ^= 1;
        assert!(
            trial_check(B_TRIAL, &mut env)
                .unwrap_err()
                .contains("no environment")
        );
        assert!(copy_valid(&env.bytes[..COPY]));
    }

    /// Gaia's real images, when they're on this machine (skipped otherwise).
    #[test]
    fn gaias_images_boot_what_their_names_say() {
        let dir = std::path::Path::new(
            "/mnt/ssd-samsung/gaia/work/.gaia/examples/luckfox-pico-pro/out/images",
        );
        let Ok(factory) = std::fs::read(dir.join("env.img")) else {
            return;
        };
        let read = |name: &str| std::fs::read(dir.join(name)).unwrap();
        let cases = [
            ("env-a.img", Slot::A, false),
            ("env-b.img", Slot::B, false),
            ("env-a-trial.img", Slot::A, true),
            ("env-b-trial.img", Slot::B, true),
        ];
        for (name, slot, trial) in cases {
            assert_eq!(
                copy_boots(&read(name)),
                Some(Running { slot, trial }),
                "{name}"
            );
        }
        assert_eq!(factory.len(), 2 * COPY);
        assert_eq!(factory[..COPY], read("env-a.img")[..]);
        assert_eq!(factory[COPY..], read("env-a.img")[..]);
        // The whole dance on the real images.
        let mut env = RamMtd::new(2, COPY as u32, 2048);
        env.bytes.copy_from_slice(&factory);
        arm_trial(A, &mut env, &read("env-b-trial.img")).unwrap();
        assert_eq!(uboot(&mut env), B_TRIAL);
        commit(B_TRIAL, &mut env, &read("env-b.img")).unwrap();
        assert_eq!(
            uboot(&mut env),
            Running {
                slot: Slot::B,
                trial: false
            }
        );
    }

    #[test]
    fn write_boot_checks_before_it_erases_the_kernel() {
        let mut env = factory();
        let mut boot = RamMtd::new(48, COPY as u32, 2048);
        let kernel = vec![7u8; 200_000];
        let wrong = write_boot(
            A,
            &mut env,
            &mut boot,
            &image(Slot::A, true),
            &mut &kernel[..],
            200_000,
            &mut |_| {},
        );
        assert!(wrong.is_err());
        assert!(boot.log.is_empty());
        write_boot(
            A,
            &mut env,
            &mut boot,
            &image(Slot::B, true),
            &mut &kernel[..],
            200_000,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(&boot.bytes[..200_000], &kernel[..]);
        assert!(
            env.log.is_empty(),
            "the env waits until the root is written too"
        );
    }
}
