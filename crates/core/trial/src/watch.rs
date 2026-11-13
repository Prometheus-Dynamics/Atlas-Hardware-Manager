//! The watch: one board, from "a trial was started" to the trial's outcome.
//!
//! Atlas does not confirm a trial. The board confirms itself (Lemnos's slot driver), or falls
//! back at the next boot, and records what it did. This watch follows the board through the
//! restarts by events:
//!
//! 1. the board leaves ([`Seen::Away`]): expected, it is restarting; never a fault;
//! 2. the board comes back as the same node ([`Seen::Found`] with the key the trial started on):
//!    the watch asks for the board's records ([`Next::ReadRecords`]);
//! 3. the records are read ([`Read`]): the board's `trial` (still settling) or its `resolved`
//!    (held, or did not hold), and the running version (`version.slot`, the identity fact) is
//!    compared with the trial's slot. A disagreement between the record and the running version
//!    is shown as both facts, never resolved by picking one.
//!
//! The outcome comes ONLY from the board's records. "It came back and looks fine" is never an
//! outcome. Nothing in this module holds a clock or a deadline: a board on a robot may take as
//! long as it takes, and a watch that has not heard from it keeps waiting ([`TrialState::NotBackYet`]).
//!
//! The states a person sees are [`TrialState`], with the board's own words carried verbatim.

use atlas_slots::{Attempt, BoardOutcome, Reported, SlotList, TrialLast};

/// A node's public key, as the session reports it.
pub type NodeKey = [u8; 32];

/// The hello fact that says which slot the board is RUNNING (`version.slot`, written at boot).
/// A confirm changes the default slot, not this one.
pub const RUNNING_FACT: &str = "version.slot";

/// What the session says about the board, mapped from its presence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Connected, identified by `key`. `running` is the board's `version.slot`, when it publishes one.
    Found {
        key: NodeKey,
        running: Option<String>,
    },
    /// Not connected now: restarting, power cut, or not reachable. Never a fault for a trial.
    Away,
    /// Another node answers at the address. The watch ends here and reads nothing from it.
    Replaced {
        expected: NodeKey,
        found: Option<NodeKey>,
    },
}

/// What the watch asks its driver to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing to read yet: wait for the next event.
    Wait,
    /// The board is back: read its slot list and give it to [`Watch::read`].
    ReadRecords,
}

/// The answer of a read of the board's records.
// One read is built per event and taken at once: boxing the list would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The board's slot list.
    List(SlotList),
    /// The board left during the read (not connected). The watch waits for its return.
    Gone(String),
    /// The board answered, but not with a list this build can read: its words, verbatim.
    Refused(String),
}

/// Which slot the board runs, against the trial's slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Running {
    /// The board runs the trial's slot: the new version.
    Trial,
    /// The board runs another slot: back on the old one. Its id, as the board says it.
    Other(String),
    /// The board did not say which slot it runs.
    NotReported,
}

/// What the board's record says a trial came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Confirmed,
    FellBack,
}

/// A trial that is still running across boots (the board's `trial`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settling {
    pub slot: String,
    /// Boot attempts left before the board falls back. `None` when the board does not say.
    pub attempts_left: Option<u32>,
    /// Boot attempts so far, each with the board's time.
    pub attempts: Vec<Attempt>,
    /// The board's word for where the trial stands, if it gave one.
    pub last: Option<TrialLast>,
    pub last_time: String,
    /// The board's reason, if it gave one (text for a fall back, otherwise empty).
    pub reason: String,
    /// Why the board could not write its record of the trial, when it could not.
    pub record_error: Option<String>,
    pub running: Running,
    /// The trial slot's signature outcome. `NotReported` on a board that does not write it (L2).
    pub signature: Reported<BoardOutcome>,
}

/// A trial the board has decided (its `resolved` record for this slot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decided {
    pub slot: String,
    /// `auto` (the board confirmed it itself) or `call`, as the board writes it.
    pub by: String,
    /// Who made the call, when it was a call.
    pub caller: String,
    /// The board's own reason, verbatim.
    pub reason: String,
    /// The board's time, verbatim.
    pub time: String,
    pub running: Running,
    /// The trial slot's signature outcome. `NotReported` on a board that does not write it (L2).
    pub signature: Reported<BoardOutcome>,
}

/// What a person sees about one board's trial. The board's words are carried verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrialState {
    /// The restart was asked for; the board has not been seen going down or coming back yet.
    Restarting { slot: String },
    /// Could not tell: the board has left and has not come back. Still waiting. The watch never
    /// gives up on its own: a person may stop it.
    NotBackYet { slot: String },
    /// The board is running the trial (its `trial` record). Attempts and the board's reason are
    /// shown as the board wrote them.
    Settling(Settling),
    /// Held: the board confirmed the trial (`resolved`, outcome confirmed), by what and when.
    Held(Decided),
    /// Did not hold: the board fell back, with its reason. `running` says which version it runs now.
    DidNotHold(Decided),
    /// Could not tell: the board's records and its running version disagree. Both facts are
    /// shown; neither is picked.
    Disagree {
        slot: String,
        record: Outcome,
        running: String,
    },
    /// Could not tell: the board came back with no `trial` and no `resolved` for this slot (an
    /// older board, or its record was lost).
    NoRecord { slot: String, running: Running },
    /// Could not tell: the board came back, but its records could not be read. The board's words.
    Unreadable { slot: String, words: String },
    /// Could not tell: another board answers at this address. The watch ends with that reason and
    /// reads nothing from the other board.
    Replaced {
        expected: String,
        found: Option<String>,
    },
    /// A person stopped following. The board's trial is not changed by this.
    Stopped { slot: String },
}

impl TrialState {
    /// Whether the watch has ended. An ended watch is not followed any more and is released from
    /// the saved watches once its state is reported.
    #[must_use]
    pub const fn finished(&self) -> bool {
        matches!(
            self,
            Self::Held(_)
                | Self::DidNotHold(_)
                | Self::Disagree { .. }
                | Self::Replaced { .. }
                | Self::Stopped { .. }
        )
    }

    /// The slot this trial is for.
    #[must_use]
    pub fn slot(&self) -> &str {
        match self {
            Self::Restarting { slot }
            | Self::NotBackYet { slot }
            | Self::Disagree { slot, .. }
            | Self::NoRecord { slot, .. }
            | Self::Unreadable { slot, .. }
            | Self::Stopped { slot } => slot,
            Self::Settling(settling) => &settling.slot,
            Self::Held(decided) | Self::DidNotHold(decided) => &decided.slot,
            Self::Replaced { .. } => "",
        }
    }

    /// Whether the board refused the trial's slot for its signature (I5g's `refused`). `false`
    /// when the board did not say, or said another thing.
    #[must_use]
    pub fn refused_by_signature(&self) -> bool {
        let signature = match self {
            Self::Settling(settling) => &settling.signature,
            Self::Held(decided) | Self::DidNotHold(decided) => &decided.signature,
            _ => return false,
        };
        matches!(signature, Reported::Known(BoardOutcome::Refused))
    }
}

/// One board's trial, followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    board: NodeKey,
    slot: String,
    started_ms: u64,
    /// The board has been seen going away since the restart began.
    away: bool,
    /// The board's running slot, as its last session said.
    running: Option<String>,
    state: TrialState,
}

impl Watch {
    /// A watch for a trial that was just started on `board`. `started_ms` is the caller's clock
    /// (milliseconds), kept only to order the saved watches.
    #[must_use]
    pub fn begin(board: NodeKey, slot: impl Into<String>, started_ms: u64) -> Self {
        let slot = slot.into();
        Self {
            board,
            away: false,
            running: None,
            state: TrialState::Restarting { slot: slot.clone() },
            slot,
            started_ms,
        }
    }

    /// A watch restored from the saved watches after Atlas restarted. The board is not connected
    /// yet, so it counts as away until it is seen.
    #[must_use]
    pub fn resume(board: NodeKey, slot: impl Into<String>, started_ms: u64) -> Self {
        let slot = slot.into();
        Self {
            board,
            away: true,
            running: None,
            state: TrialState::NotBackYet { slot: slot.clone() },
            slot,
            started_ms,
        }
    }

    /// The board this trial is on.
    #[must_use]
    pub const fn board(&self) -> NodeKey {
        self.board
    }

    /// The trial's slot id.
    #[must_use]
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// When the trial was started, in the caller's milliseconds.
    #[must_use]
    pub const fn started_ms(&self) -> u64 {
        self.started_ms
    }

    /// What a person sees now.
    #[must_use]
    pub const fn state(&self) -> &TrialState {
        &self.state
    }

    /// Whether the watch has ended (see [`TrialState::finished`]).
    #[must_use]
    pub const fn finished(&self) -> bool {
        self.state.finished()
    }

    /// Whether the board has been seen going away since the restart began.
    #[must_use]
    pub const fn seen_away(&self) -> bool {
        self.away
    }

    /// Takes one event from the session. Returns what to do next.
    pub fn seen(&mut self, seen: Seen) -> Next {
        if self.finished() {
            return Next::Wait;
        }
        match seen {
            Seen::Replaced { expected, found } => {
                self.end_replaced(expected, found);
                Next::Wait
            }
            Seen::Found { key, .. } if key != self.board => {
                self.end_replaced(self.board, Some(key));
                Next::Wait
            }
            Seen::Found { running, .. } => {
                self.running = running.filter(|id| !id.is_empty());
                Next::ReadRecords
            }
            Seen::Away => {
                self.away = true;
                self.state = TrialState::NotBackYet {
                    slot: self.slot.clone(),
                };
                Next::Wait
            }
        }
    }

    /// Takes the board's answer to a read of its records (asked for after [`Next::ReadRecords`]).
    pub fn read(&mut self, read: Read) {
        if self.finished() {
            return;
        }
        match read {
            Read::Gone(_) => {
                self.away = true;
                self.state = TrialState::NotBackYet {
                    slot: self.slot.clone(),
                };
            }
            Read::Refused(words) => {
                self.state = TrialState::Unreadable {
                    slot: self.slot.clone(),
                    words,
                };
            }
            Read::List(list) => self.state = self.decide(&list),
        }
    }

    /// The person stopped following. Nothing on the board changes.
    pub fn stop(&mut self) {
        if !self.finished() {
            self.state = TrialState::Stopped {
                slot: self.slot.clone(),
            };
        }
    }

    fn end_replaced(&mut self, expected: NodeKey, found: Option<NodeKey>) {
        self.state = TrialState::Replaced {
            expected: hex(&expected),
            found: found.map(|key| hex(&key)),
        };
    }

    fn running(&self) -> Running {
        match self.running.as_deref() {
            None => Running::NotReported,
            Some(id) if id == self.slot => Running::Trial,
            Some(id) => Running::Other(id.to_owned()),
        }
    }

    fn decide(&self, list: &SlotList) -> TrialState {
        let slot = self.slot.clone();
        let running = self.running();
        // LOCK: Lemnos L2 (I5g): the per-slot `signature` word. Until the board writes it, an absent
        // word is NotReported, never verified.
        let signature = list
            .get(&self.slot)
            .map_or(Reported::NotReported, |listed| listed.signature.clone());

        if let Some(trial) = list.trial.as_ref() {
            if trial.slot != self.slot {
                return TrialState::Unreadable {
                    slot,
                    words: format!(
                        "the board reports a trial of {}, not of {}",
                        trial.slot, self.slot
                    ),
                };
            }
            return TrialState::Settling(Settling {
                slot,
                attempts_left: trial.attempts_left,
                attempts: trial.attempts.clone(),
                last: trial.last.clone(),
                last_time: trial.last_time.clone(),
                reason: trial.reason.clone(),
                record_error: trial.record_error.clone(),
                running,
                signature,
            });
        }

        let Some(resolved) = list
            .resolved
            .as_ref()
            .filter(|resolved| resolved.slot == self.slot)
        else {
            return TrialState::NoRecord { slot, running };
        };
        let outcome = match &resolved.outcome {
            atlas_slots::Resolution::Confirmed => Outcome::Confirmed,
            atlas_slots::Resolution::FellBack => Outcome::FellBack,
            atlas_slots::Resolution::Other(word) => {
                return TrialState::Unreadable {
                    slot,
                    words: format!("the board's outcome {word:?} is not one this build reads"),
                };
            }
        };
        let decided = Decided {
            slot: slot.clone(),
            by: resolved.by.clone(),
            caller: resolved.caller.clone(),
            reason: resolved.reason.clone(),
            time: resolved.time.clone(),
            running: running.clone(),
            signature,
        };
        // The record and the running version are two facts. When they disagree, both are shown.
        match (outcome, running) {
            (Outcome::Confirmed, Running::Other(running)) => TrialState::Disagree {
                slot,
                record: Outcome::Confirmed,
                running,
            },
            (Outcome::FellBack, Running::Trial) => TrialState::Disagree {
                slot,
                record: Outcome::FellBack,
                running: self.slot.clone(),
            },
            (Outcome::Confirmed, _) => TrialState::Held(decided),
            (Outcome::FellBack, _) => TrialState::DidNotHold(decided),
        }
    }
}

/// A node key as text: lowercase hex, 64 digits.
#[must_use]
pub fn hex(key: &NodeKey) -> String {
    use std::fmt::Write;
    key.iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// A node key from its 64 hex digits, or `None` when the text is not one.
#[must_use]
pub fn key_from_hex(text: &str) -> Option<NodeKey> {
    if text.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}
