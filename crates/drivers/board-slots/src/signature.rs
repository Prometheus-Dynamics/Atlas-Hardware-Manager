//! The signature outcome of a slot, in Atlas's own words.
//!
//! The board writes one of four outcomes per slot (its `signature` word, the table first flash and
//! slot installs share, read by `atlas_slots` as [`atlas_slots::BoardOutcome`]). Atlas says each one
//! to a person in these words, and a refusal carries the board's reason unchanged: Atlas adds
//! nothing to what the board refused for.
//!
//! - `verified`: a key the board holds signed the slot, and it verifies.
//! - `not_checked_unsigned`: the slot has no signature. Installed (the security switch is off).
//! - `not_checked_unknown_key`: a key the board does not hold signed it. Installed, not checked.
//! - `refused`: the board refused it; the reason is the board's, verbatim.

// LOCK: Lemnos L2: the per-slot `signature` word is read from the board's list only once the L2 rev
// writes it (today's lock writes `signed` only, and `atlas_slots` reports NotReported without it).

use atlas_slots::{BoardOutcome, Reported, Slot};

/// One outcome, as Atlas says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureWords {
    /// `verified`.
    Verified,
    /// `not_checked_unsigned`.
    NotCheckedUnsigned,
    /// `not_checked_unknown_key`.
    NotCheckedUnknownKey,
    /// `refused`, with the board's reason exactly as it sent it.
    Refused { reason: String },
}

impl SignatureWords {
    /// The outcome the board gave, with the board's reason for a refusal. An outcome this Atlas does
    /// not know is `None`: it is never shown as one of the four.
    #[must_use]
    pub fn from_board(outcome: &BoardOutcome, reason: &str) -> Option<Self> {
        match outcome {
            BoardOutcome::Verified => Some(Self::Verified),
            BoardOutcome::NotCheckedUnsigned => Some(Self::NotCheckedUnsigned),
            BoardOutcome::NotCheckedUnknownKey => Some(Self::NotCheckedUnknownKey),
            BoardOutcome::Refused => Some(Self::Refused {
                reason: reason.to_string(),
            }),
            BoardOutcome::Unrecognised(_) => None,
        }
    }

    /// The outcome of a slot in a board's list. `None` when the board does not report it, or
    /// reports a word this Atlas does not know.
    #[must_use]
    pub fn from_slot(slot: &Slot) -> Option<Self> {
        match &slot.signature {
            Reported::Known(outcome) => Self::from_board(outcome, ""),
            Reported::NotReported => None,
        }
    }

    /// The board's wire word for the outcome.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::NotCheckedUnsigned => "not_checked_unsigned",
            Self::NotCheckedUnknownKey => "not_checked_unknown_key",
            Self::Refused { .. } => "refused",
        }
    }

    /// The sentence a person reads. A refusal is the board's reason, unchanged.
    #[must_use]
    pub fn plain(&self) -> String {
        match self {
            Self::Verified => "verified: signed by a key this board holds".into(),
            Self::NotCheckedUnsigned => "not checked: the slot is unsigned".into(),
            Self::NotCheckedUnknownKey => {
                "not checked: signed by a key this board does not hold".into()
            }
            Self::Refused { reason } => reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_outcomes_have_their_board_words_and_plain_sentences() {
        let cases = [
            (BoardOutcome::Verified, "verified"),
            (BoardOutcome::NotCheckedUnsigned, "not_checked_unsigned"),
            (
                BoardOutcome::NotCheckedUnknownKey,
                "not_checked_unknown_key",
            ),
            (BoardOutcome::Refused, "refused"),
        ];
        for (outcome, word) in cases {
            let words = SignatureWords::from_board(&outcome, "the reason").expect("known");
            assert_eq!(words.word(), word);
            assert!(!words.plain().is_empty());
        }
    }

    #[test]
    fn a_refusal_carries_the_boards_reason_unchanged() {
        let reason = "-1: slot.sig is signed by a key this board holds, and it does not verify slot.json: refused, nothing is written";
        let words = SignatureWords::from_board(&BoardOutcome::Refused, reason).expect("known");
        assert_eq!(words.plain(), reason);
    }

    #[test]
    fn an_outcome_this_atlas_does_not_know_is_not_one_of_the_four() {
        let unknown = BoardOutcome::Unrecognised("maybe".into());
        assert_eq!(SignatureWords::from_board(&unknown, ""), None);
    }
}
