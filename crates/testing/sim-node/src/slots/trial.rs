//! The trial lifecycle (Lemnos I5f): a trial boot of a slot, the board's own confirm of a healthy
//! trial, its fall back at the deadline or after boots that never reach the daemon, and the record
//! `list` reports as `trial` and `resolved`.
//!
//! Time is the simulated clock ([`super::Slots::now_s`]), moved by the test with
//! `SimNode::slot_clock_advance`, so a trial resolves without any wall-clock wait. Every boot is an
//! explicit event: a trial boot happens when the board is restarted (`trial`, the reboot action, or
//! [`super::Slots::reboot`]). A boot that never reaches the daemon leaves the board on the default
//! slot with the trial still pending, as a boot that dies before lemnosd leaves it; the next restart
//! boots the trial again. Nothing restarts the board on its own.
//!
//! What is not modelled: the `health` list (the sim has one healthy flag instead), the
//! no-regression check on devices, the boot partition's counter files (the sim keeps the counts in
//! [`Record`]), and the `trial-reason` file's read at the next start.

use serde_json::{Value, json};

use super::{Slots, State, Status, code};

/// The `system-slots` config keys of I5f, with the brief's defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The board confirms a healthy trial itself. Off: only `confirm` confirms, and nothing
    /// restarts at a deadline.
    pub auto_confirm: bool,
    /// How long the daemon must have run in the trial before it may confirm.
    pub confirm_uptime_s: u64,
    /// Time since the daemon started by which every condition must hold.
    pub confirm_deadline_s: u64,
    /// At the deadline with a condition unmet: record why, and restart into the default.
    pub reboot_at_deadline: bool,
    /// Boots in a row that may fail to reach the daemon before the trial falls back.
    pub trial_attempts: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_confirm: true,
            confirm_uptime_s: 30,
            confirm_deadline_s: 600,
            reboot_at_deadline: true,
            trial_attempts: 3,
        }
    }
}

/// What the latest boot of a pending trial did (`last` in `list`). A resolved trial is not
/// pending, so its outcome is reported as `resolved` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Last {
    PowerLostBeforeDaemon,
    DaemonReachedNotConfirmed,
}

impl Last {
    const fn label(self) -> &'static str {
        match self {
            Self::PowerLostBeforeDaemon => "power-lost-before-daemon",
            Self::DaemonReachedNotConfirmed => "daemon-reached-not-yet-confirmed",
        }
    }
}

/// One trial boot, numbered from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub n: u32,
    pub time_s: u64,
}

/// The record of a pending trial: its boots and what they did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Record {
    pub attempts: Vec<Attempt>,
    /// Boots of this trial that never reached the daemon. A boot that reached it gives its
    /// attempt back, so it never counts here.
    pub without_daemon: u32,
    /// Whether the boot now running reached the daemon.
    pub reached: bool,
    /// The simulated time the daemon started in the boot that reached it.
    pub daemon_since_s: u64,
    pub last: Option<Last>,
    pub last_time_s: u64,
    /// Why the trial is not confirmed, when a deadline passed with a condition unmet and the
    /// trial was kept (`reboot_at_deadline` off).
    pub reason: Option<String>,
    /// Set when the driver could not write its record of the trial.
    pub record_error: Option<String>,
}

/// Who decided a resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum By {
    /// The board, on its own rules.
    Auto,
    /// A call, by this caller.
    Call(String),
}

/// The last resolution of a trial (`resolved` in `list`). It survives the fallback boot, and is
/// replaced by the next trial's resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub slot: String,
    pub confirmed: bool,
    pub by: By,
    pub reason: String,
    pub time_s: u64,
}

// LOCK: Lemnos L2 (the raw two-copy boot-partition records, I5i). Today: the boot record has
// `trial_armed` only, and no attempt counter is written to the boot partition.
// LOCK: Lemnos L1 (trial-reason and record_error, I5f3). Today: no trial-reason file, and no
// record_error in `list`.
/// The reason written when the driver cannot write its record.
const RECORD_FAILED: &str = "the state directory refused the write";
/// The reason a trial is not confirmed while the board is not healthy.
const UNHEALTHY: &str = "a health condition is not met: the board is not healthy";

/// The simulated time as text. Fixed-format, from the start of the simulated clock.
fn stamp(secs: u64) -> String {
    format!("T+{secs}s")
}

impl Slots {
    /// Moves the simulated clock on by `secs`, then runs the trial's rules.
    pub fn advance_clock(&mut self, secs: u64) {
        self.now_s = self.now_s.saturating_add(secs);
        self.run_timers();
        self.evaluate();
    }

    /// Sets whether the record writes fail. A pending trial carries the error from now on.
    pub fn set_record_writes_fail(&mut self, fail: bool) {
        self.fail_record_writes = fail;
        self.write_record();
    }

    /// The daemon was restarted: its uptime starts again, so a trial's confirm waits for it.
    pub fn daemon_restarted(&mut self) {
        let now = self.now_s;
        if let Some(trial) = self.trial.as_mut().filter(|trial| trial.record.reached) {
            trial.record.daemon_since_s = now;
        }
    }

    /// The power goes while the trial's boot runs and the daemon has started. The board is on the
    /// default slot until the next restart, which boots the trial again. No attempt is spent: the
    /// boot reached the daemon.
    pub fn power_cut(&mut self) {
        self.boot_default();
        self.state = State::Idle;
        if let Some(trial) = self.trial.as_mut() {
            trial.record.reached = false;
        }
    }

    /// Runs the trial's own rules on the clock. A healthy trial whose daemon has run for
    /// `confirm_uptime_s` confirms. At `confirm_deadline_s` with a condition still unmet, the reason
    /// is recorded and the board restarts into the default (or only records it, when
    /// `reboot_at_deadline` is off). Cheap when there is no trial: the node runs it on every tick.
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    pub fn evaluate(&mut self) {
        let config = self.trial_config;
        let now = self.now_s;
        let Some(trial) = self.trial.as_ref() else {
            return;
        };
        if !trial.record.reached || !config.auto_confirm {
            return;
        }
        let uptime = now.saturating_sub(trial.record.daemon_since_s);
        let id = trial.id.clone();
        let unmet = self.unmet();
        if uptime >= config.confirm_uptime_s && unmet.is_none() {
            self.confirm_trial(
                By::Auto,
                format!("healthy for {uptime} s since the daemon started"),
            );
            return;
        }
        if uptime < config.confirm_deadline_s {
            return;
        }
        let reason = unmet.unwrap_or_else(|| "the trial was not confirmed by its deadline".into());
        if config.reboot_at_deadline {
            self.fall_back(&id, &reason, By::Auto);
        } else if let Some(trial) = self.trial.as_mut() {
            trial.record.reason = Some(reason);
        }
    }

    /// A restart: the reboot action, the first boot of a trial, or a boot after a power cut.
    ///
    /// A trial that reached the daemon and did not confirm falls back to the default, as a restart
    /// always did. A trial that has not reached the daemon boots again. With no trial, the default
    /// runs.
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    pub fn reboot(&mut self) {
        self.state = State::Idle;
        match self.trial.as_ref() {
            Some(trial) if trial.record.reached => {
                let id = trial.id.clone();
                self.fall_back(
                    &id,
                    "the trial did not confirm before the next restart",
                    By::Auto,
                );
            }
            Some(_) => self.boot_trial(),
            None => self.boot_default(),
        }
    }

    /// Confirms the running slot, as the `confirm` action does. A trial the board is running is
    /// confirmed (see the module docs). A pending trial the board is not running is cleared, as a
    /// fall back, by this call. With no trial, the running slot is marked good.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND` when the running slot is unknown.
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    pub fn confirm(&mut self, caller: &str) -> Result<(), Status> {
        let active = self
            .active
            .clone()
            .ok_or_else(|| Status::new(code::NOT_FOUND, "the running slot is unknown"))?;
        let running_trial = self
            .trial
            .as_ref()
            .is_some_and(|trial| trial.record.reached && trial.id == active);
        if running_trial {
            self.confirm_trial(
                By::Call(caller.to_string()),
                format!("confirmed by a call from {caller}"),
            );
            return Ok(());
        }
        if let Some(trial) = self.trial.take() {
            let reason =
                format!("the board runs {active}, not the trial: cleared by a call from {caller}");
            self.fail(&trial.id, "the board is not running the trial");
            self.mark_good(&active);
            self.state = State::RolledBack;
            self.resolved = Some(Resolved {
                slot: trial.id,
                confirmed: false,
                by: By::Call(caller.to_string()),
                reason,
                time_s: self.now_s,
            });
            return Ok(());
        }
        self.mark_good(&active);
        self.state = State::Confirmed;
        Ok(())
    }

    /// The `trial` and `resolved` fields of `list`.
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    pub(super) fn trial_json(&self) -> Value {
        let Some(trial) = self.trial.as_ref() else {
            return Value::Null;
        };
        let record = &trial.record;
        let attempts_left = self
            .trial_config
            .trial_attempts
            .saturating_sub(record.without_daemon);
        json!({
            "slot": trial.id,
            "attempts_left": attempts_left,
            "attempts": record.attempts.iter()
                .map(|attempt| json!({"n": attempt.n, "time": stamp(attempt.time_s)}))
                .collect::<Vec<_>>(),
            "last": record.last.map(Last::label),
            "last_time": record.last.map(|_| stamp(record.last_time_s)),
            "reason": record.reason,
            "record_error": record.record_error,
        })
    }

    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    pub(super) fn resolved_json(&self) -> Value {
        let Some(resolved) = self.resolved.as_ref() else {
            return Value::Null;
        };
        let (by, caller) = match &resolved.by {
            By::Auto => ("auto", Value::Null),
            By::Call(caller) => ("call", Value::from(caller.clone())),
        };
        json!({
            "slot": resolved.slot,
            "outcome": if resolved.confirmed { "confirmed" } else { "fell-back" },
            "by": by,
            "caller": caller,
            "reason": resolved.reason,
            "time": stamp(resolved.time_s),
        })
    }

    /// The reason the trial cannot confirm now, if one holds.
    fn unmet(&self) -> Option<String> {
        if let Some(error) = self
            .trial
            .as_ref()
            .and_then(|trial| trial.record.record_error.as_ref())
        {
            return Some(format!("the state could not be written: {error}"));
        }
        if !self.healthy {
            return Some(UNHEALTHY.into());
        }
        None
    }

    /// One boot of the pending trial. It either reaches the daemon, or does not: a boot that does
    /// not spends an attempt, and after `trial_attempts` of them the trial falls back. A boot that
    /// does not reach the daemon leaves the board on the default slot with the trial pending.
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    fn boot_trial(&mut self) {
        // A trial whose boot files differ from its manifest fails by the normal failure path.
        if let Some(id) = self.trial.as_ref().map(|trial| trial.id.clone())
            && let Some(reason) = self.why(&id)
        {
            self.fall_back(&id, &reason, By::Auto);
            return;
        }
        let now = self.now_s;
        let lost = self.boots_without_daemon > 0;
        if lost {
            self.boots_without_daemon -= 1;
        }
        let allowed = self.trial_config.trial_attempts;
        let Some(trial) = self.trial.as_mut() else {
            return;
        };
        let number = u32::try_from(trial.record.attempts.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        trial.record.attempts.push(Attempt {
            n: number,
            time_s: now,
        });
        trial.record.last_time_s = now;
        if lost {
            trial.record.without_daemon += 1;
            trial.record.reached = false;
            trial.record.last = Some(Last::PowerLostBeforeDaemon);
        } else {
            trial.record.reached = true;
            trial.record.daemon_since_s = now;
            trial.record.last = Some(Last::DaemonReachedNotConfirmed);
        }
        let id = trial.id.clone();
        let failed_boots = trial.record.without_daemon;
        self.write_record();
        if !lost {
            self.active = Some(id.clone());
            self.event(format!("trial boot of {id}"));
        } else if failed_boots >= allowed {
            let reason =
                format!("the trial slot did not reach a running daemon in {failed_boots} boots");
            self.fall_back(&id, &reason, By::Auto);
        } else {
            self.boot_default();
            self.state = State::Idle;
            self.event(format!("trial boot of {id} did not reach the daemon"));
        }
    }

    /// Confirms the pending trial: it becomes the default (unless it was started to keep the
    /// default), the old default becomes previous, and housekeeping runs (unless it keeps old).
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    fn confirm_trial(&mut self, by: By, reason: String) {
        let Some(trial) = self.trial.take() else {
            return;
        };
        let active = trial.id.clone();
        if !trial.keep_default {
            self.previous = self.default.replace(active.clone());
        }
        self.mark_good(&active);
        self.state = State::Confirmed;
        self.event(format!("{active} confirmed"));
        if !trial.keep_old {
            self.housekeeping();
        }
        self.trial_reason = None;
        self.resolved = Some(Resolved {
            slot: active,
            confirmed: true,
            by,
            reason,
            time_s: self.now_s,
        });
    }

    /// The trial falls back: the slot is marked failed, the board runs the default, and the
    /// resolution keeps the reason. When the record cannot be written, the reason also goes to the
    /// boot partition's `trial-reason`.
    // LOCK: Lemnos L1 (I5f). Pre-L1 board: a trial is confirmed only by an explicit confirm; no resolved, no record_error.
    fn fall_back(&mut self, id: &str, reason: &str, by: By) {
        self.fail(id, reason);
        self.trial = None;
        self.boot_default();
        self.state = State::RolledBack;
        self.trial_reason = self.fail_record_writes.then(|| reason.to_string());
        self.resolved = Some(Resolved {
            slot: id.to_string(),
            confirmed: false,
            by,
            reason: reason.to_string(),
            time_s: self.now_s,
        });
    }

    /// Writes the pending trial's record: it carries the error while the writes fail.
    // LOCK: Lemnos L1 (I5f3, record_error). Pre-L1 board: no record_error in `list`.
    fn write_record(&mut self) {
        let failing = self.fail_record_writes;
        if let Some(trial) = self.trial.as_mut() {
            trial.record.record_error = failing.then(|| RECORD_FAILED.to_string());
        }
    }
}
