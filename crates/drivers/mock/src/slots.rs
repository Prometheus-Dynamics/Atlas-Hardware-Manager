//! The root slots, with the rules of Lemnos's slot driver (I5): good and failed marks,
//! default and previous, a trial boot that falls back when it does not confirm, guarded
//! remove, pins, keep and housekeeping, and the install gate. Pure data and rules, so each
//! rule is checked on its own. The node runs calls against it.

use serde_json::json;

/// Lemnos statuses (`lemnos-abi`): 0 is ok, negative values are errors.
pub mod code {
    pub const NOT_PERMITTED: i32 = -1;
    pub const NOT_FOUND: i32 = -2;
    pub const INTERRUPTED: i32 = -4;
    pub const IO: i32 = -5;
    pub const BUSY: i32 = -16;
    pub const NO_DEVICE: i32 = -19;
    pub const INVALID_ARGUMENT: i32 = -22;
    pub const ENOSPC: i32 = -28;
    pub const CANCELLED: i32 = -125;
}

/// A failure with its Lemnos status and a sentence for people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub code: i32,
    pub message: String,
}

impl Status {
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Whether a slot was good at its last confirm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Good {
    Yes,
    No,
    Untried,
}

impl Good {
    fn label(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Untried => "untried",
        }
    }
}

/// How a slot's signature stood when it was installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signed {
    /// Checked against a trusted key, which belongs to `owner`.
    Trusted { owner: String },
    /// Signed, by a key that is not trusted.
    Untrusted,
    /// No signature in the bundle.
    Unsigned,
}

/// A key the board trusts, with whom it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedKey {
    pub public: [u8; 32],
    pub owner: String,
}

/// What the slot driver is doing, as `list` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Staging,
    Error,
    Cancelled,
    Rebooting,
    Confirmed,
    RolledBack,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Staging => "staging",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
            Self::Rebooting => "rebooting",
            Self::Confirmed => "confirmed",
            Self::RolledBack => "rolled-back",
        }
    }
}

/// One installed slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub family: String,
    pub version: String,
    pub variant: String,
    pub kernel: String,
    pub display_name: String,
    pub size: u64,
    pub verity: bool,
    pub signed: Signed,
    pub good: Good,
    pub failed: bool,
    pub failures: u32,
    pub fail_reason: String,
    pub pinned: bool,
    /// Installs get increasing numbers, so `list` can show the newest first.
    pub seq: u64,
}

/// A trial boot: a slot to boot once, to be confirmed or fallen back from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    pub id: String,
    pub keep_default: bool,
    pub keep_old: bool,
    /// Whether the board has booted it. The next restart without a confirm falls back.
    pub booted: bool,
}

/// The facts of an installed bundle, as the install gate hands them over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSlot {
    pub id: String,
    pub family: String,
    pub version: String,
    pub variant: String,
    pub kernel: String,
    pub display_name: String,
    pub size: u64,
    pub verity: bool,
    pub signed: Signed,
}

/// The slot state of one board.
#[derive(Clone, Debug)]
pub struct Slots {
    pub entries: Vec<Entry>,
    /// The slot the board runs.
    pub active: Option<String>,
    pub default: Option<String>,
    pub previous: Option<String>,
    pub trial: Option<Trial>,
    pub state: State,
    pub error: String,
    /// Slots to keep. Zero keeps all.
    pub keep: u32,
    pub free_bytes: u64,
    pub reserve_bytes: u64,
    /// The security switch. Off, signatures are shown but do not refuse an install.
    pub require_signed: bool,
    pub trusted: Vec<TrustedKey>,
    /// What happened, for the log, drained by the model.
    pub events: Vec<String>,
    seq: u64,
}

const RESERVE_BYTES: u64 = 2048 * 1024 * 1024;

impl Default for Slots {
    fn default() -> Self {
        Self::new()
    }
}

impl Slots {
    /// A board with one good slot, running and default, and 8 GiB free.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: vec![Entry {
                id: "slot-1".into(),
                family: "sim".into(),
                version: "1.0.0".into(),
                variant: String::new(),
                kernel: String::new(),
                display_name: "Simulated board".into(),
                size: 4096,
                verity: true,
                signed: Signed::Unsigned,
                good: Good::Yes,
                failed: false,
                failures: 0,
                fail_reason: String::new(),
                pinned: false,
                seq: 1,
            }],
            active: Some("slot-1".into()),
            default: Some("slot-1".into()),
            previous: None,
            trial: None,
            state: State::Idle,
            error: String::new(),
            keep: 3,
            free_bytes: 8 * 1024 * 1024 * 1024,
            reserve_bytes: RESERVE_BYTES,
            require_signed: false,
            trusted: Vec::new(),
            events: Vec::new(),
            seq: 1,
        }
    }

    /// The slot called `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.id == id)
    }

    fn event(&mut self, text: String) {
        self.events.push(text);
    }

    /// Whether `id` is the running, default, trial or previous slot.
    fn protected(&self, id: &str) -> bool {
        self.active.as_deref() == Some(id)
            || self.default.as_deref() == Some(id)
            || self.previous.as_deref() == Some(id)
            || self.trial.as_ref().is_some_and(|trial| trial.id == id)
    }

    /// The install gate: refused while another install runs (the caller checks that), when
    /// security needs trusted keys and there are none, and when the bundle will not fit.
    ///
    /// # Errors
    ///
    /// The Lemnos status of the refusal.
    pub fn check_install_start(&self, size: u64) -> Result<(), Status> {
        if self.require_signed && self.trusted.is_empty() {
            return Err(Status::new(
                code::NOT_PERMITTED,
                "security needs a trusted key, and the board has none",
            ));
        }
        if size.saturating_add(self.reserve_bytes) > self.free_bytes {
            return Err(Status::new(
                code::ENOSPC,
                format!(
                    "the bundle is {size} bytes and the board keeps {} free",
                    self.reserve_bytes
                ),
            ));
        }
        Ok(())
    }

    /// The gate on a finished bundle: an untrusted or unsigned one is refused when security
    /// is on, and leaves nothing behind.
    ///
    /// # Errors
    ///
    /// `NOT_PERMITTED` when security refuses it.
    pub fn check_signed(&self, signed: &Signed) -> Result<(), Status> {
        match signed {
            Signed::Trusted { .. } => Ok(()),
            _ if self.require_signed => Err(Status::new(
                code::NOT_PERMITTED,
                "security is on and the bundle is not signed by a trusted key",
            )),
            _ => Ok(()),
        }
    }

    /// Adds an installed slot. It starts untried, and housekeeping runs unless `keep_old`.
    ///
    /// # Errors
    ///
    /// `BUSY` when the id is the running or default slot, `INVALID_ARGUMENT` when the id is
    /// taken by another slot.
    pub fn install_done(
        &mut self,
        new: NewSlot,
        pin: bool,
        keep_old: bool,
    ) -> Result<String, Status> {
        if self.active.as_deref() == Some(new.id.as_str())
            || self.default.as_deref() == Some(new.id.as_str())
        {
            return Err(Status::new(
                code::BUSY,
                format!("{} is the running or the default slot", new.id),
            ));
        }
        if self.get(&new.id).is_some() {
            return Err(Status::new(
                code::INVALID_ARGUMENT,
                format!("{} is already installed", new.id),
            ));
        }
        self.seq += 1;
        let id = new.id.clone();
        self.event(format!("installed {id}"));
        self.entries.push(Entry {
            id: new.id,
            family: new.family,
            version: new.version,
            variant: new.variant,
            kernel: new.kernel,
            display_name: new.display_name,
            size: new.size,
            verity: new.verity,
            signed: new.signed,
            good: Good::Untried,
            failed: false,
            failures: 0,
            fail_reason: String::new(),
            pinned: pin,
            seq: self.seq,
        });
        self.state = State::Idle;
        self.error.clear();
        if !keep_old {
            self.housekeeping();
        }
        Ok(id)
    }

    /// Makes `id` the default. The old default becomes previous.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND`, or `NOT_PERMITTED` when the slot is not good, or failed and not forced.
    pub fn set_default(&mut self, id: &str, force: bool) -> Result<(), Status> {
        let entry = self.get(id).ok_or_else(|| not_found(id))?;
        if entry.good != Good::Yes {
            return Err(Status::new(
                code::NOT_PERMITTED,
                format!("{id} is not marked good"),
            ));
        }
        if entry.failed && !force {
            return Err(Status::new(
                code::NOT_PERMITTED,
                format!("{id} failed since its last confirm"),
            ));
        }
        if self.default.as_deref() != Some(id) {
            self.previous = self.default.replace(id.to_string());
        }
        self.event(format!("default is {id}"));
        Ok(())
    }

    /// Sets a trial boot and restarts the board into it. The trial is booted now; the next
    /// restart without a confirm falls back to the default.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND`, `BUSY` when the slot is running, or `NOT_PERMITTED` when it failed and
    /// the call is not forced.
    pub fn trial(
        &mut self,
        id: &str,
        keep_default: bool,
        keep_old: bool,
        force: bool,
    ) -> Result<(), Status> {
        let entry = self.get(id).ok_or_else(|| not_found(id))?;
        if self.active.as_deref() == Some(id) {
            return Err(Status::new(code::BUSY, format!("{id} is the running slot")));
        }
        if entry.failed && !force {
            return Err(Status::new(
                code::NOT_PERMITTED,
                format!("{id} failed and the call is not forced"),
            ));
        }
        self.trial = Some(Trial {
            id: id.to_string(),
            keep_default,
            keep_old,
            booted: false,
        });
        self.state = State::Rebooting;
        self.reboot();
        Ok(())
    }

    /// Confirms the running slot. A booted trial becomes the default, unless it was started
    /// to keep the default. A trial the board is not running is recorded as failed.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND` when the running slot is unknown.
    pub fn confirm(&mut self) -> Result<(), Status> {
        let active = self
            .active
            .clone()
            .ok_or_else(|| Status::new(code::NOT_FOUND, "the running slot is unknown"))?;
        match self.trial.clone() {
            Some(trial) if trial.booted && trial.id == active => {
                self.trial = None;
                if !trial.keep_default {
                    self.previous = self.default.replace(active.clone());
                }
                self.mark_good(&active);
                self.state = State::Confirmed;
                self.event(format!("{active} confirmed"));
                if !trial.keep_old {
                    self.housekeeping();
                }
            }
            Some(trial) => {
                self.fail(&trial.id, "the board is not running the trial");
                self.trial = None;
                self.mark_good(&active);
                self.state = State::RolledBack;
            }
            None => {
                self.mark_good(&active);
                self.state = State::Confirmed;
            }
        }
        Ok(())
    }

    /// Removes a slot. The running, default, trial and only slots are busy; a pinned slot
    /// is not removed.
    ///
    /// # Errors
    ///
    /// `INVALID_ARGUMENT` for a bad id, `NOT_FOUND`, `BUSY`, or `NOT_PERMITTED` when pinned.
    pub fn remove(&mut self, id: &str) -> Result<(), Status> {
        if !valid_id(id) {
            return Err(Status::new(
                code::INVALID_ARGUMENT,
                format!("{id:?} is not a slot id"),
            ));
        }
        let entry = self.get(id).ok_or_else(|| not_found(id))?;
        if entry.pinned {
            return Err(Status::new(code::NOT_PERMITTED, format!("{id} is pinned")));
        }
        if self.entries.len() == 1
            || self.active.as_deref() == Some(id)
            || self.default.as_deref() == Some(id)
            || self.trial.as_ref().is_some_and(|trial| trial.id == id)
        {
            return Err(Status::new(code::BUSY, format!("{id} is in use")));
        }
        self.entries.retain(|entry| entry.id != id);
        if self.previous.as_deref() == Some(id) {
            self.previous = None;
        }
        self.event(format!("removed {id}"));
        Ok(())
    }

    /// Pins or unpins a slot. A pinned slot is never removed or housekept.
    ///
    /// # Errors
    ///
    /// `NOT_FOUND` when there is no such slot.
    pub fn pin(&mut self, id: &str, pinned: bool) -> Result<(), Status> {
        let entry = self.get_mut(id).ok_or_else(|| not_found(id))?;
        entry.pinned = pinned;
        Ok(())
    }

    /// Sets how many slots to keep (zero keeps all), and runs housekeeping.
    pub fn set_keep(&mut self, keep: u32) {
        self.keep = keep;
        self.housekeeping();
    }

    /// The running slot's trial fell back, or a trial failed: the slot is marked failed.
    fn fail(&mut self, id: &str, reason: &str) {
        if let Some(entry) = self.get_mut(id) {
            entry.good = Good::No;
            entry.failed = true;
            entry.failures += 1;
            entry.fail_reason = reason.to_string();
        }
        self.event(format!("{id} failed: {reason}"));
    }

    fn mark_good(&mut self, id: &str) {
        if let Some(entry) = self.get_mut(id) {
            entry.good = Good::Yes;
            entry.failed = false;
        }
    }

    /// A restart. A trial that has not booted boots now. A trial that booted and was not
    /// confirmed falls back to the default and is recorded as failed. Otherwise the default
    /// runs.
    pub fn reboot(&mut self) {
        self.state = State::Idle;
        match self.trial.take() {
            Some(trial) if trial.booted => {
                self.fail(
                    &trial.id,
                    "the trial did not confirm before the next restart",
                );
                self.active = self.default.clone();
                self.state = State::RolledBack;
            }
            Some(mut trial) => {
                trial.booted = true;
                self.active = Some(trial.id.clone());
                self.event(format!("trial boot of {}", trial.id));
                self.trial = Some(trial);
            }
            None => self.active = self.default.clone(),
        }
    }

    /// A service restart cut an install short: nothing it wrote survives, and `list` says so.
    pub fn interrupted(&mut self) {
        self.state = State::Error;
        self.error = "an install was interrupted: the service restarted".into();
        self.event(self.error.clone());
    }

    /// An install was cancelled before it wrote a slot.
    pub fn cancelled(&mut self) {
        self.state = State::Cancelled;
        self.event("install cancelled".into());
    }

    /// Starts an install's state: the driver is staging.
    pub fn staging(&mut self) {
        self.state = State::Staging;
        self.error.clear();
    }

    /// Keeps the running, default, trial and previous slots, then the newest slots, up to
    /// `keep`. Pinned slots never count and never go. Zero keeps everything.
    fn housekeeping(&mut self) {
        if self.keep == 0 {
            return;
        }
        let mut count = self
            .entries
            .iter()
            .filter(|entry| !entry.pinned && self.protected(&entry.id))
            .count();
        let mut candidates: Vec<(u64, String)> = self
            .entries
            .iter()
            .filter(|entry| !entry.pinned && !self.protected(&entry.id))
            .map(|entry| (entry.seq, entry.id.clone()))
            .collect();
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
        for (_, id) in candidates {
            if count < self.keep as usize {
                count += 1;
            } else {
                self.entries.retain(|entry| entry.id != id);
                self.event(format!("kept last {}: removed {id}", self.keep));
            }
        }
    }

    fn signed_label(&self, signed: &Signed) -> &'static str {
        match signed {
            Signed::Trusted { .. } => "trusted",
            _ if !self.require_signed => "unverified",
            Signed::Untrusted => "untrusted",
            Signed::Unsigned => "unsigned",
        }
    }

    /// `list`, as the slot driver writes it: newest slot first.
    #[must_use]
    pub fn list_json(&self) -> String {
        let mut ordered: Vec<&Entry> = self.entries.iter().collect();
        ordered.sort_by_key(|entry| std::cmp::Reverse(entry.seq));
        let slots: Vec<serde_json::Value> = ordered
            .iter()
            .map(|entry| {
                let mut flags = Vec::new();
                if self.active.as_deref() == Some(entry.id.as_str()) {
                    flags.push("running");
                }
                if self.default.as_deref() == Some(entry.id.as_str()) {
                    flags.push("default");
                }
                if self.previous.as_deref() == Some(entry.id.as_str()) {
                    flags.push("previous");
                }
                if self
                    .trial
                    .as_ref()
                    .is_some_and(|trial| trial.id == entry.id)
                {
                    flags.push("trial");
                }
                let signer = match &entry.signed {
                    Signed::Trusted { owner } => owner.clone(),
                    _ => String::new(),
                };
                json!({
                    "id": entry.id,
                    "family": entry.family,
                    "version": entry.version,
                    "variant": entry.variant,
                    "kernel": entry.kernel,
                    "display_name": entry.display_name,
                    "good": entry.good.label(),
                    "signed": self.signed_label(&entry.signed),
                    "signer": signer,
                    "installed": "",
                    "last_boot": "",
                    "size": entry.size,
                    "verity": entry.verity,
                    "intact": true,
                    "flags": flags,
                    "pinned": entry.pinned,
                    "failures": entry.failures,
                    "failed": entry.failed,
                })
            })
            .collect();
        json!({
            "slot_active": self.active.clone().unwrap_or_default(),
            "slot_default": self.default.clone().unwrap_or_default(),
            "slot_previous": self.previous.clone().unwrap_or_default(),
            "slot_trial": self.trial.as_ref().map(|trial| trial.id.clone()).unwrap_or_default(),
            "slot_staged": "",
            "state": self.state.label(),
            "error": self.error,
            "free_bytes": self.free_bytes,
            "reserve_bytes": self.reserve_bytes,
            "keep": self.keep,
            "slots": slots,
        })
        .to_string()
    }
}

fn not_found(id: &str) -> Status {
    Status::new(code::NOT_FOUND, format!("there is no slot {id}"))
}

/// A slot id: letters, digits, `.`, `_` and `-`, not `.` or `..`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
