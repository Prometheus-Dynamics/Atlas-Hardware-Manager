//! The signature outcome of a slot, as the board words it.
//!
//! The board's words are one table for first flash and slot installs (Lemnos I5g). Atlas keeps
//! the word the board gave, and only the words in this table are known. A word this build does
//! not know is kept as text, never read as verified.

use std::fmt;

/// The outcome of a signature check, as the board's `signature` field names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureOutcome {
    /// Signed, and the signature verifies with a key on the board's list.
    Verified,
    /// Unsigned; the signature was not checked.
    NotCheckedUnsigned,
    /// Signed by a key the board does not hold; the signature was not checked.
    NotCheckedUnknownKey,
    /// A word this build does not know, kept as the board wrote it.
    Other(String),
}

// MERGE: use atlas-image's BoardOutcome when PR 56 (origin/pr/D15) lands. Until then this is the
// seam: first flash and slot installs share the same four words, and nothing else in Atlas
// interprets them.
pub type BoardOutcome = SignatureOutcome;

impl SignatureOutcome {
    /// Reads the board's word.
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        match word {
            "verified" => Self::Verified,
            "not_checked_unsigned" => Self::NotCheckedUnsigned,
            "not_checked_unknown_key" => Self::NotCheckedUnknownKey,
            other => Self::Other(other.to_owned()),
        }
    }

    /// The board's word for this outcome.
    #[must_use]
    pub fn word(&self) -> &str {
        match self {
            Self::Verified => "verified",
            Self::NotCheckedUnsigned => "not_checked_unsigned",
            Self::NotCheckedUnknownKey => "not_checked_unknown_key",
            Self::Other(word) => word,
        }
    }

    /// Whether the signature was checked and verified.
    #[must_use]
    pub const fn is_verified(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

impl fmt::Display for SignatureOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}
