//! How long a detached run may run on the node, per kind of run: one table, in code and in
//! `docs/prs/PINS-2.md`. A node ends a run at its deadline (`TimedOut`, code 110 at this pin), and the user
//! is told in plain words which kind and which bound (`overrun_words`).
//!
//! | kind | what the run does on the board | what bounds it | deadline |
//! |---|---|---|---|
//! | `Install { bytes }` (`install_file`) | writes the bundle into the slot's bay, then reads it back to verify | the payload size the caller knows, times a stated minimum write rate, plus the read-back, plus a fixed overhead | `install_bound(bytes)`: floor 5 min, ceiling 4 h |
//! | `Generic` (a board action job, `ActionJobs`) | whatever the action does; Atlas does not know its length | nothing Atlas knows today: Lemnos's per-job stall bound (M40) is not in the pin | `GENERIC_INTERIM`, 1 h, an interim value pending the ruling |
//! | trial (`BoardSlots::start_trial`) | nothing detached: the trial start is a direct, attached call that returns at once, and the trial then follows by the slot list | the board's own confirm deadline (`confirm_deadline_s`), which Atlas does not read | not a detached run, so no row here; the bound is missing (see the doc) |

use std::time::Duration;

/// The minimum write rate Atlas accepts for a slot bay: a bundle is written at no less than this (1 MiB/s, a
/// conservative floor for the SD and eMMC bays in use). Stated, for ruling.
pub const MIN_WRITE_BYTES_PER_SEC: u64 = 1024 * 1024;
/// The read-back verify rate: 4 MiB/s, a floor for reading the bay back. Stated, for ruling.
pub const VERIFY_BYTES_PER_SEC: u64 = 4 * 1024 * 1024;
/// What the board does around the write that does not depend on the size: the bundle check, the slot record and
/// the list refresh. Two minutes, stated, for ruling.
pub const INSTALL_OVERHEAD: Duration = Duration::from_secs(120);
/// The shortest install bound: a small bundle still gets five minutes.
pub const INSTALL_FLOOR: Duration = Duration::from_secs(5 * 60);
/// The longest install bound: four hours, so a stuck write cannot hold the lease and the slot longer than that.
pub const INSTALL_CEILING: Duration = Duration::from_secs(4 * 60 * 60);
/// The interim bound of a generic board action (an action Atlas knows nothing about): one hour. Interim, pending
/// the ruling and Lemnos's per-job stall bound (M40).
pub const GENERIC_INTERIM: Duration = Duration::from_secs(60 * 60);

/// The kind of a detached run, which decides its deadline and its words when it expires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    /// An install of a bundle of `bytes` into a bay.
    Install { bytes: u64 },
    /// A generic board action, a job Atlas started without knowing its length.
    Generic,
}

/// The deadline of an install of `bytes`: the fixed overhead, plus the write at the minimum rate, plus the
/// read-back at the verify rate, held between the floor and the ceiling.
#[must_use]
pub fn install_bound(bytes: u64) -> Duration {
    let write = bytes.div_ceil(MIN_WRITE_BYTES_PER_SEC);
    let verify = bytes.div_ceil(VERIFY_BYTES_PER_SEC);
    let total = INSTALL_OVERHEAD
        .saturating_add(Duration::from_secs(write))
        .saturating_add(Duration::from_secs(verify));
    total.clamp(INSTALL_FLOOR, INSTALL_CEILING)
}

/// The deadline of a run of `kind`.
#[must_use]
pub fn bound_of(kind: RunKind) -> Duration {
    match kind {
        RunKind::Install { bytes } => install_bound(bytes),
        RunKind::Generic => GENERIC_INTERIM,
    }
}

/// The deadline of a run of `kind`, in milliseconds, as the node takes it (`deadline_ms`).
#[must_use]
pub fn deadline_ms(kind: RunKind) -> u64 {
    u64::try_from(bound_of(kind).as_millis()).unwrap_or(u64::MAX)
}

/// The plain line the user sees when a run of `kind` named `action` expires at its deadline: which kind, and the
/// bound it had. The node ended it (`TimedOut`, 110).
#[must_use]
pub fn overrun_words(kind: RunKind, action: &str) -> String {
    let minutes = bound_of(kind).as_secs().div_ceil(60);
    match kind {
        RunKind::Install { .. } => {
            format!("the install did not finish within {minutes} minutes; the board ended it")
        }
        RunKind::Generic => {
            format!("{action} did not finish within {minutes} minutes; the board ended it")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_install_gets_the_floor_and_a_large_one_the_derived_bound() {
        // 28 160 bytes (the sim's bundle): overhead 2 min + a second to write and to verify = 2 min 2 s, so the floor.
        assert_eq!(install_bound(28_160), INSTALL_FLOOR);
        // 1 GiB: 120 s + 1024 s to write + 256 s to verify = 1400 s (23 min 20 s).
        assert_eq!(
            install_bound(1024 * 1024 * 1024),
            Duration::from_secs(120 + 1024 + 256)
        );
        // Far beyond the ceiling: capped.
        assert_eq!(install_bound(u64::MAX / 2), INSTALL_CEILING);
    }

    #[test]
    fn the_generic_interim_is_one_hour_and_every_kind_has_a_positive_deadline() {
        assert_eq!(bound_of(RunKind::Generic), GENERIC_INTERIM);
        assert!(deadline_ms(RunKind::Generic) > 0);
        assert!(
            deadline_ms(RunKind::Install { bytes: 0 }) > 0,
            "no detached run without a deadline"
        );
        assert_eq!(deadline_ms(RunKind::Generic), 3_600_000);
    }

    #[test]
    fn an_expired_install_and_an_expired_action_say_their_kind_and_bound_in_plain_words() {
        assert_eq!(
            overrun_words(RunKind::Install { bytes: 28_160 }, "install_file"),
            "the install did not finish within 5 minutes; the board ended it"
        );
        assert_eq!(
            overrun_words(RunKind::Generic, "system.long"),
            "system.long did not finish within 60 minutes; the board ended it"
        );
    }
}
