//! The board's slot list (`system.slots/1` `list`), read into data.
//!
//! The board writes one JSON document (Lemnos I5-actions.md, "list JSON", with the I5f
//! `trial` and `resolved` objects and the I5g per-slot `signature` word). Slots come newest
//! first. The reading is strict where a missing value would mislead, and lenient where the
//! board may not have written the field yet:
//!
//! - An absent per-slot `signature` is [`Reported::NotReported`], never verified: a board that
//!   does not say cannot be taken to have checked a signature.
//! - An unknown `signature` or `last` word is kept as text ([`BoardOutcome::Other`]).
//! - An absent `trial` or `resolved` is "none" (`None`), which is what the board writes when
//!   there is nothing pending.
//! - A top-level document without `slots` is refused: that is not a list.

use serde_json::Value;
use thiserror::Error;

use atlas_image::BoardOutcome;

/// A field of the board's answer. [`Reported::NotReported`] is shown as "not reported".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reported<T> {
    Known(T),
    NotReported,
}

/// One installed slot as the board lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub id: String,
    pub product: String,
    /// The board the slot was built for. A slot for another board is listed, and refused for
    /// set-default and trial by the board.
    pub board: String,
    pub version: String,
    pub variant: String,
    pub kernel: String,
    pub display_name: String,
    /// `yes`, `no` or `untried` as written.
    pub good: String,
    /// `trusted`, `untrusted` or `unsigned` as written (the label; see [`Slot::signature`]).
    pub signed: String,
    /// The owner of the key that signed it, when trusted.
    pub signer: String,
    pub size: u64,
    pub verity: bool,
    pub flags: Vec<String>,
    /// The board's pin. This is the one notion of "starred" Atlas has: see [`crate::policy`].
    pub pinned: bool,
    pub failures: u32,
    pub failed: bool,
    /// The signature outcome (I5g, the shared table). Absent on a board that does not write it yet.
    pub signature: Reported<BoardOutcome>,
}

impl Slot {
    /// Whether the slot is the running, default, trial or staged slot (the board's flags).
    #[must_use]
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|found| found == flag)
    }

    /// Whether the person has starred this version. Starred is the board's pin.
    #[must_use]
    pub const fn starred(&self) -> bool {
        self.pinned
    }
}

/// Where a pending trial last stood, as the board words it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrialLast {
    PowerLostBeforeDaemon,
    DaemonReachedNotYetConfirmed,
    Confirmed,
    FellBack,
    /// A word this build does not know.
    Other(String),
}

/// One boot attempt of a pending trial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub n: u32,
    pub time: String,
}

/// A trial that is pending across boots (the board's `trial` object).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    pub slot: String,
    pub attempts_left: Option<u32>,
    pub attempts: Vec<Attempt>,
    pub last: Option<TrialLast>,
    pub last_time: String,
    pub reason: String,
    /// Why the board could not write its record of the trial, when it could not.
    pub record_error: Option<String>,
}

/// How a resolved trial was resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Confirmed,
    FellBack,
    Other(String),
}

/// The last trial outcome (the board's `resolved` object). It stays until the next trial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub slot: String,
    pub outcome: Resolution,
    /// `auto` (the board confirmed it itself) or `call` (someone asked).
    pub by: String,
    pub caller: String,
    pub reason: String,
    pub time: String,
}

/// The board's slot list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotList {
    /// The running slot's id, or empty.
    pub running: String,
    pub default: String,
    pub previous: String,
    pub trial_slot: String,
    pub staged: String,
    /// `idle`, `staging`, `error`, ... as written.
    pub state: String,
    /// The board's last error sentence, or empty.
    pub error: String,
    pub free_bytes: u64,
    pub reserve_bytes: u64,
    /// The board's keep count (N). Zero keeps all.
    pub keep: u32,
    /// Newest first, as the board lists them.
    pub slots: Vec<Slot>,
    pub trial: Option<Trial>,
    pub resolved: Option<Resolved>,
}

/// Why a board's answer is not a slot list.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ListError {
    #[error("the board's slot list is not JSON: {0}")]
    NotJson(String),
    #[error("the board's slot list has no slots field")]
    NoSlots,
    #[error("slot {index} in the board's list has no id")]
    NoId { index: usize },
}

impl SlotList {
    /// Reads the board's answer.
    ///
    /// # Errors
    ///
    /// [`ListError`] when the text is not JSON, has no `slots` array, or a slot has no id.
    pub fn parse(text: &str) -> Result<Self, ListError> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| ListError::NotJson(error.to_string()))?;
        let slots_value = value.get("slots").and_then(Value::as_array).ok_or(ListError::NoSlots)?;
        let mut slots = Vec::with_capacity(slots_value.len());
        for (index, entry) in slots_value.iter().enumerate() {
            slots.push(parse_slot(entry, index)?);
        }
        Ok(Self {
            running: text_of(&value, "slot_active"),
            default: text_of(&value, "slot_default"),
            previous: text_of(&value, "slot_previous"),
            trial_slot: text_of(&value, "slot_trial"),
            staged: text_of(&value, "slot_staged"),
            state: text_of(&value, "state"),
            error: text_of(&value, "error"),
            free_bytes: num_of(&value, "free_bytes"),
            reserve_bytes: num_of(&value, "reserve_bytes"),
            keep: u32::try_from(num_of(&value, "keep")).unwrap_or(u32::MAX),
            slots,
            trial: parse_trial(value.get("trial")),
            resolved: parse_resolved(value.get("resolved")),
        })
    }

    /// The slot with this id, if the board lists it.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.id == id)
    }
}

fn parse_slot(entry: &Value, index: usize) -> Result<Slot, ListError> {
    let id = text_of(entry, "id");
    if id.is_empty() {
        return Err(ListError::NoId { index });
    }
    let flags = entry
        .get("flags")
        .and_then(Value::as_array)
        .map(|flags| {
            flags
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let signature = match entry.get("signature").and_then(Value::as_str) {
        Some(word) if !word.is_empty() => Reported::Known(BoardOutcome::parse(word)),
        _ => Reported::NotReported,
    };
    Ok(Slot {
        id,
        product: text_of(entry, "product"),
        board: text_of(entry, "board"),
        version: text_of(entry, "version"),
        variant: text_of(entry, "variant"),
        kernel: text_of(entry, "kernel"),
        display_name: text_of(entry, "display_name"),
        good: text_of(entry, "good"),
        signed: text_of(entry, "signed"),
        signer: text_of(entry, "signer"),
        size: num_of(entry, "size"),
        verity: entry.get("verity").and_then(Value::as_bool).unwrap_or(false),
        flags,
        pinned: entry.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        failures: u32::try_from(num_of(entry, "failures")).unwrap_or(u32::MAX),
        failed: entry.get("failed").and_then(Value::as_bool).unwrap_or(false),
        signature,
    })
}

fn parse_trial(value: Option<&Value>) -> Option<Trial> {
    let value = value.filter(|value| value.is_object())?;
    let attempts = value
        .get("attempts")
        .and_then(Value::as_array)
        .map(|attempts| {
            attempts
                .iter()
                .map(|attempt| Attempt {
                    n: u32::try_from(num_of(attempt, "n")).unwrap_or(u32::MAX),
                    time: text_of(attempt, "time"),
                })
                .collect()
        })
        .unwrap_or_default();
    let last = value
        .get("last")
        .and_then(Value::as_str)
        .map(|word| match word {
            "power-lost-before-daemon" => TrialLast::PowerLostBeforeDaemon,
            "daemon-reached-not-yet-confirmed" => TrialLast::DaemonReachedNotYetConfirmed,
            "confirmed" => TrialLast::Confirmed,
            "fell-back" => TrialLast::FellBack,
            other => TrialLast::Other(other.to_owned()),
        });
    let record_error = match text_of(value, "record_error") {
        text if text.is_empty() => None,
        text => Some(text),
    };
    Some(Trial {
        slot: text_of(value, "slot"),
        attempts_left: value
            .get("attempts_left")
            .and_then(Value::as_u64)
            .map(|left| u32::try_from(left).unwrap_or(u32::MAX)),
        attempts,
        last,
        last_time: text_of(value, "last_time"),
        reason: text_of(value, "reason"),
        record_error,
    })
}

fn parse_resolved(value: Option<&Value>) -> Option<Resolved> {
    let value = value.filter(|value| value.is_object())?;
    let outcome = match text_of(value, "outcome").as_str() {
        "confirmed" => Resolution::Confirmed,
        "fell-back" => Resolution::FellBack,
        other => Resolution::Other(other.to_owned()),
    };
    // The caller is an id: text or a number, as the board writes it.
    let caller = match value.get("caller") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    Some(Resolved {
        slot: text_of(value, "slot"),
        outcome,
        by: text_of(value, "by"),
        caller,
        reason: text_of(value, "reason"),
        time: text_of(value, "time"),
    })
}

fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn num_of(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}
