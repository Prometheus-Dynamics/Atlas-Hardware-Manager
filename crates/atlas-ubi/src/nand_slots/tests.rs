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

#[test]
fn the_commit_image_survives_on_data_and_only_the_right_one_is_kept() {
    let dir = std::env::temp_dir().join(format!("atlas-ubi-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(kept_commit_image(&dir, Slot::B), None);
    assert!(keep_commit_image(&dir, Slot::B, &image(Slot::A, false)).is_err());
    assert!(keep_commit_image(&dir, Slot::B, &image(Slot::B, true)).is_err());
    assert_eq!(kept_commit_image(&dir, Slot::B), None);
    keep_commit_image(&dir, Slot::B, &image(Slot::B, false)).unwrap();
    assert_eq!(
        kept_commit_image(&dir, Slot::B),
        Some(image(Slot::B, false))
    );
    assert_eq!(kept_commit_image(&dir, Slot::A), None);
    // Damaged on /data: not used; the commit asks for the image again.
    std::fs::write(kept_path(&dir, Slot::B), [0u8; COPY]).unwrap();
    assert_eq!(kept_commit_image(&dir, Slot::B), None);
    forget_commit_image(&dir, Slot::B);
    assert!(!kept_path(&dir, Slot::B).exists());
    let _ = std::fs::remove_dir_all(dir);
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
