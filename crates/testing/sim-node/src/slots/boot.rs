//! The boot-file rules, as Lemnos's code does them (next 0c74ec76; the texts and the Pi form from
//! e3e8a963: slot-init `boot.rs`, lemnosd `boot_guard.rs`, slot-store `bootcheck.rs`). Every boot
//! checks the files the manifest lists, before the image is opened; the default is also checked
//! before every restart and every 600 s; a failed check moves the default to its previous slot when
//! that one matches, and otherwise a fallback starts another slot on trial.
//!
//! The simulated clock is `Slots::now_s`. Nothing waits in real time: a rescue is a deadline on
//! the clock, moved by `advance_clock`.

use std::collections::BTreeMap;

use super::{Slots, State, Status, Trial, code, trial};

// LOCK: Lemnos L2 (boot.rs, boot_guard.rs at e3e8a963; the gaps at 0c74ec76). The texts in this
// module are Lemnos's, verbatim (INPUTS, "Boot-file mismatch, AS LEMNOS'S CODE DOES IT" and "the gaps
// closed"). Lemnos checks each file in the manifest's list order; this model keeps the manifest map's
// order (sorted by path). At the lock move slot-init and lemnosd run these rules; this module is their
// model.

/// slot-init's `rescue_wait` default, on the simulated clock (confirmed at 0c74ec76).
pub const RESCUE_WAIT_S: u64 = 30;
/// lemnosd's period for the default's check, on the simulated clock ("every ten minutes").
pub const PERIODIC_CHECK_S: u64 = 600;
/// Fallbacks in a row without a confirmed boot, after which no fallback starts.
pub const MAX_FALLBACKS_IN_A_ROW: u32 = 3;

/// Each manifest file, in the manifest's order, that cannot be read, is missing from the boot
/// partition, or whose hash differs: bootcheck's three texts (`<name> can't be read: <why>`,
/// `<name> is missing from the boot partition`, `<name> doesn't match its manifest hash`).
fn mismatches(
    manifest: &BTreeMap<String, String>,
    disk: &BTreeMap<String, String>,
    unreadable: &BTreeMap<String, String>,
) -> Vec<String> {
    manifest
        .iter()
        .filter_map(|(name, wanted)| {
            if let Some(why) = unreadable.get(name) {
                return Some(format!("{name} can't be read: {why}"));
            }
            match disk.get(name) {
                None => Some(format!("{name} is missing from the boot partition")),
                Some(found) if found != wanted => {
                    Some(format!("{name} doesn't match its manifest hash"))
                }
                Some(_) => None,
            }
        })
        .collect()
}

impl Slots {
    /// Why slot `id`'s boot files do not match its manifest: the per-file list joined with `"; "`.
    /// `None` when they all match (or the slot is not installed).
    #[must_use]
    pub fn why(&self, id: &str) -> Option<String> {
        let entry = self.get(id)?;
        let list = mismatches(
            &entry.boot_manifest,
            &entry.boot_disk,
            &entry.boot_unreadable,
        );
        (!list.is_empty()).then(|| list.join("; "))
    }

    /// Sets a boot file's digest as it sits on the boot partition (`None`: the file is gone). A test
    /// makes a file differ from its manifest this way.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND` when there is no such slot.
    pub fn set_boot_file(
        &mut self,
        id: &str,
        path: &str,
        digest: Option<&str>,
    ) -> Result<(), Status> {
        let entry = self
            .get_mut(id)
            .ok_or_else(|| Status::new(code::NOT_FOUND, format!("there is no slot {id}")))?;
        match digest {
            Some(digest) => entry.boot_disk.insert(path.to_string(), digest.to_string()),
            None => entry.boot_disk.remove(path),
        };
        Ok(())
    }

    /// Makes a boot file unreadable to the store, with the reader's reason (`Some`), or readable
    /// again (`None`). An unreadable file is reported as `<name> can't be read: <why>`.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND` when there is no such slot.
    pub fn set_boot_file_unreadable(
        &mut self,
        id: &str,
        path: &str,
        why: Option<&str>,
    ) -> Result<(), Status> {
        let entry = self
            .get_mut(id)
            .ok_or_else(|| Status::new(code::NOT_FOUND, format!("there is no slot {id}")))?;
        match why {
            Some(why) => entry
                .boot_unreadable
                .insert(path.to_string(), why.to_string()),
            None => entry.boot_unreadable.remove(path),
        };
        Ok(())
    }

    /// The default's check (boot_guard.rs `check`). When the default does not match and the previous
    /// matches, the previous becomes the default (the old default is marked failed first) and the
    /// note `<default> doesn't match its manifest (<why>): <previous> is the default now` is returned.
    /// When neither matches, the text is an error: with `restart` it ends `: the restart is refused`,
    /// without it (the periodic check) the same text only reports.
    ///
    /// # Errors
    ///
    /// The text above, when neither the default nor its previous matches.
    fn default_check(&mut self, restart: bool) -> Result<Option<String>, String> {
        let Some(id) = self.default.clone() else {
            return Ok(None);
        };
        let Some(why) = self.why(&id) else {
            return Ok(None);
        };
        let previous = self.previous.clone().unwrap_or_default();
        if !previous.is_empty() && self.get(&previous).is_some() && self.why(&previous).is_none() {
            self.repoint(&id, &previous, &why);
            return Ok(Some(format!(
                "{id} doesn't match its manifest ({why}): {previous} is the default now"
            )));
        }
        let text = format!(
            "{id} doesn't match its manifest ({why}), and its previous {previous:?} doesn't either"
        );
        Err(if restart {
            format!("{text}: the restart is refused")
        } else {
            text
        })
    }

    /// Makes `previous` the default: the old default is marked failed with the repoint's reason, then
    /// the two pointers swap. Lemnos's `repoint`; the prefix is written only here.
    fn repoint(&mut self, id: &str, previous: &str, why: &str) {
        if let Some(entry) = self.get_mut(id) {
            entry.failed = true;
            entry.fail_reason = format!("its boot files don't match its manifest: {why}");
        }
        self.default = Some(previous.to_string());
        self.previous = Some(id.to_string());
        self.event(format!(
            "{id} is marked failed: its boot files don't match its manifest"
        ));
    }

    /// The restart check, before a restart the board performs (lemnosd). Returns the note when the
    /// default was repointed and the restart proceeds. When neither the default nor its previous
    /// matches, the restart is refused by name and the board stays up.
    ///
    /// # Errors
    ///
    /// `NOT_PERMITTED` with the text from [`Slots::default_check`].
    pub fn check_restart(&mut self) -> Result<Option<String>, Status> {
        match self.default_check(true) {
            Ok(note) => {
                if let Some(note) = &note {
                    self.event(note.clone());
                }
                Ok(note)
            }
            Err(text) => Err(Status::new(code::NOT_PERMITTED, text)),
        }
    }

    /// The periodic check (lemnosd, at once and every [`PERIODIC_CHECK_S`]): the same check, with no
    /// restart. It repoints when the previous matches; otherwise it only reports.
    pub(super) fn periodic_check(&mut self) {
        match self.default_check(false) {
            Ok(Some(note)) => self.event(note),
            Ok(None) => {}
            Err(text) => self.event(text),
        }
    }

    /// The timers of the simulated clock: a rescue whose wait ran out restarts the board (a plain
    /// restart, which boots again), and the periodic check runs when its period is due.
    pub(super) fn run_timers(&mut self) {
        if let Some(at) = self.rescue_at
            && self.now_s >= at
        {
            self.rescue_at = None;
            self.boot_default();
        }
        if self.now_s >= self.next_check_s {
            self.next_check_s = self.now_s.saturating_add(PERIODIC_CHECK_S);
            self.periodic_check();
        }
    }

    /// The boot path (slot-init, every boot before the image opens). The default runs when its files
    /// match. When they do not:
    /// - a previous that matches takes its place at once, with no count;
    /// - no valid previous: the ordinary failure path (the default fails with the bare reason, then
    ///   another startable slot starts on trial, or no slot can start);
    /// - a previous that does not match either: no slot can start, and no pointer moves.
    pub(super) fn boot_default(&mut self) {
        let Some(id) = self.default.clone() else {
            self.active = None;
            return;
        };
        let Some(why) = self.why(&id) else {
            self.active = Some(id);
            self.rescue_at = None;
            return;
        };
        let previous = self.previous.clone().unwrap_or_default();
        let previous_exists = !previous.is_empty() && self.get(&previous).is_some();
        if previous_exists && self.why(&previous).is_none() {
            self.repoint(&id, &previous, &why);
            self.event(format!(
                "slot {id}'s boot files don't match its manifest: {why}; {previous} is the default now, restarting into it"
            ));
            self.active = Some(previous);
            self.rescue_at = None;
            return;
        }
        if !previous_exists {
            self.event(format!("slot {id} can't start: {why}"));
            self.fail(&id, &why);
            self.start_fallback(&id, &why);
            return;
        }
        let previous_why = self.why(&previous).unwrap_or_default();
        self.no_slot(&format!(
            "{why}; its previous {previous} doesn't match its manifest either: {previous_why}"
        ));
    }

    /// The ordinary failure path's fallback: another startable slot starts on trial (its fallback
    /// count goes up), unless three fallbacks have come in a row without a confirmed boot.
    fn start_fallback(&mut self, failed: &str, why: &str) {
        if self.fallbacks_in_a_row >= MAX_FALLBACKS_IN_A_ROW {
            self.no_slot(&format!(
                "{why}; fell back {MAX_FALLBACKS_IN_A_ROW} times in a row without a confirmed boot"
            ));
            return;
        }
        let candidate = self
            .entries
            .iter()
            .filter(|entry| entry.id != failed && !entry.failed && self.why(&entry.id).is_none())
            .max_by_key(|entry| entry.seq)
            .map(|entry| entry.id.clone());
        let Some(next) = candidate else {
            self.no_slot(&format!("{why}; no other slot can start"));
            return;
        };
        self.fallbacks_in_a_row += 1;
        self.event(format!(
            "falling back: restarting into slot {next} on trial"
        ));
        self.trial = Some(Trial {
            id: next,
            keep_default: true,
            keep_old: true,
            record: trial::Record::default(),
        });
        self.reboot();
    }

    /// No slot can start: the text, the wait, and a plain restart when the wait runs out.
    fn no_slot(&mut self, reason: &str) {
        let text = format!("no slot can start: {reason}");
        self.event(text.clone());
        self.event(format!("restarting in {} s", self.rescue_wait_s));
        self.active = None;
        self.state = State::Error;
        self.error = text;
        self.rescue_at = Some(self.now_s.saturating_add(self.rescue_wait_s));
    }
}
