//! The boot checks of Lemnos I5i4: a slot's boot files (kernel, device tree, overlays, cmdline.txt)
//! are the ones its manifest names. Two refusals, which the sim models:
//!
//! - a BOOT whose files differ from the manifest does not run: slot-init refuses it. The default
//!   that does not boot leaves the previous slot to boot when its files match, else no slot runs
//!   and the board says why; a trial whose files differ falls back at once.
//! - a RESTART (the reboot action, or a trial call) whose target differs from its manifest is
//!   refused by lemnosd, by name, before the board goes down.
//!
//! Only the sim's boot-partition copy is compared (`boot_disk` against `boot_manifest`); no bytes
//! are read.

// LOCK: Lemnos L2 (I5i4). Today Lemnos has neither refusal: a boot runs the slot it is told to, and a
// restart is not checked against the slot's manifest. At the lock move slot-init refuses the boot and
// lemnosd refuses the restart, both by name; this module is their model.
use std::collections::BTreeMap;

use super::{Slots, State, Status, code};

/// The first difference between a manifest's boot files and the ones on the boot partition, named
/// by path. `None` when they agree.
fn difference(
    manifest: &BTreeMap<String, String>,
    disk: &BTreeMap<String, String>,
) -> Option<String> {
    let paths: std::collections::BTreeSet<&String> = manifest.keys().chain(disk.keys()).collect();
    for path in paths {
        let wanted = manifest.get(path);
        let found = disk.get(path);
        if wanted != found {
            let describe = |digest: Option<&String>| {
                digest.map_or("nothing".to_string(), |d| format!("sha256 {d}"))
            };
            return Some(format!(
                "boot/{path} is {} on the boot partition, and slot.json says {}",
                describe(found),
                describe(wanted)
            ));
        }
    }
    None
}

impl Slots {
    /// Why the slot `id` would not boot, when its boot files differ from its manifest.
    #[must_use]
    pub fn boot_mismatch(&self, id: &str) -> Option<String> {
        let entry = self.get(id)?;
        difference(&entry.boot_manifest, &entry.boot_disk)
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

    /// Refuses a restart whose boot would run a slot that differs from its manifest: the pending
    /// trial when it has not reached the daemon, else the default. Nothing restarts when refused.
    ///
    /// # Errors
    ///
    /// `NOT_PERMITTED`, naming the file.
    pub fn check_restart(&self) -> Result<(), Status> {
        let target = match &self.trial {
            Some(trial) if !trial.record.reached => Some(trial.id.clone()),
            _ => self.default.clone(),
        };
        match target {
            Some(id) => self.refuse_restart_into(&id),
            None => Ok(()),
        }
    }

    /// The restart check for a boot into `id`.
    pub(super) fn refuse_restart_into(&self, id: &str) -> Result<(), Status> {
        match self.boot_mismatch(id) {
            None => Ok(()),
            Some(reason) => Err(Status::new(
                code::NOT_PERMITTED,
                format!("restart refused: {id} would boot with {reason}; nothing restarted"),
            )),
        }
    }

    /// The boot path: the default runs. A default whose files differ does not boot (the previous
    /// slot does, when its files match); with neither, no slot runs and `error` says why.
    pub(super) fn boot_default(&mut self) {
        let Some(id) = self.default.clone() else {
            self.active = None;
            return;
        };
        match self.boot_mismatch(&id) {
            None => self.active = Some(id),
            Some(reason) => {
                self.event(format!("{id} refused at boot: {reason}"));
                match self.previous.clone() {
                    Some(previous) if self.boot_mismatch(&previous).is_none() => {
                        self.event(format!("{previous} boots instead"));
                        self.active = Some(previous);
                    }
                    _ => {
                        self.active = None;
                        self.state = State::Error;
                        self.error = format!("{id} does not boot: {reason}");
                    }
                }
            }
        }
    }
}
