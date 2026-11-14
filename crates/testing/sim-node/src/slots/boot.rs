//! The boot-file rules, as Lemnos's code does them (next e3e8a963: slot-init `boot.rs`, lemnosd
//! `boot_guard.rs`, slot-store `bootcheck.rs`). Every boot checks the files the manifest lists,
//! before the image is opened; the default is also checked before every restart and every ten
//! minutes; a failed check moves the default to its previous slot when that one matches.
//!
//! The simulated clock is `Slots::now_s`. Nothing waits in real time: a rescue is a deadline on
//! the clock, moved by `advance_clock`.

use std::collections::BTreeMap;

use super::{Slots, State, Status, code};

// LOCK: Lemnos L2 (boot.rs, boot_guard.rs at e3e8a963). The texts in this module are Lemnos's,
// verbatim (INPUTS, "Boot-file mismatch, AS LEMNOS'S CODE DOES IT"). Lemnos checks each file in the
// manifest's list order; this model keeps the manifest map's order (sorted by path). At the lock move
// slot-init and lemnosd run these rules, and this module is their model.

/// slot-init's `rescue_wait` default, on the simulated clock.
pub const RESCUE_WAIT_S: u64 = 30;
/// lemnosd's period for the default's check, on the simulated clock ("every ten minutes").
pub const PERIODIC_CHECK_S: u64 = 600;

/// Each manifest file that is missing from the boot partition or whose hash differs, in the
/// manifest's order: `<name> is missing from the boot partition` or `<name> doesn't match its
/// manifest hash` (bootcheck's `Mismatch` texts).
fn mismatches(manifest: &BTreeMap<String, String>, disk: &BTreeMap<String, String>) -> Vec<String> {
    manifest
        .iter()
        .filter_map(|(name, wanted)| match disk.get(name) {
            None => Some(format!("{name} is missing from the boot partition")),
            Some(found) if found != wanted => {
                Some(format!("{name} doesn't match its manifest hash"))
            }
            Some(_) => None,
        })
        .collect()
}

impl Slots {
    /// Why slot `id`'s boot files do not match its manifest: the per-file list joined with `"; "`.
    /// `None` when they all match (or the slot is not installed).
    #[must_use]
    pub fn why(&self, id: &str) -> Option<String> {
        let entry = self.get(id)?;
        let list = mismatches(&entry.boot_manifest, &entry.boot_disk);
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

    /// Makes `previous` the default: the old default is marked failed (with its reason), then the
    /// two pointers swap. Lemnos's `repoint`.
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

    /// The periodic check (lemnosd, every [`PERIODIC_CHECK_S`]): the same check, no restart. A
    /// failed check only reports.
    pub(super) fn periodic_check(&mut self) {
        match self.default_check(false) {
            Ok(Some(note)) => self.event(note),
            Ok(None) => {}
            Err(text) => self.event(text),
        }
    }

    /// The timers of the simulated clock: a rescue whose wait ran out restarts the board, and the
    /// periodic check runs when its period is due.
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
    /// match. When they do not: a previous that matches takes its place at once, with no count; with
    /// neither matching, no slot can start, no pointer moves, and the board restarts after the rescue
    /// wait. A default with no valid previous is failed by the normal failure path.
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
            // No valid previous: the default fails by the normal failure path.
            self.fail(&id, &why);
        }
        let text = if previous_exists {
            match self.why(&previous) {
                Some(why2) => format!(
                    "no slot can start: {why}; its previous {previous} doesn't match its manifest either: {why2}"
                ),
                None => format!("no slot can start: {why}"),
            }
        } else {
            format!("no slot can start: {why}")
        };
        self.event(text.clone());
        self.active = None;
        self.state = State::Error;
        self.error = text;
        self.rescue_at = Some(self.now_s.saturating_add(self.rescue_wait_s));
    }
}
