//! What kind of refusal a call on a live session was, carried as a value. Orion's error is not
//! matched by its words: a caller that must act on a refusal (a key the node does not know, for
//! one) asks [`Failure::kind`], and the text is kept beside it for people to read.

use std::fmt;

use orion::types::Error as CoreError;
use orion::{ClientError, Error as OrionError};

/// The kinds of Orion refusal Atlas acts on. A variant Atlas does not name is [`FailureKind::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The node turned this client away: Orion's `Denied` (a session refused at hello or on the
    /// key, or a call refused by the comm core's check, `denied`). For a board this is a key the
    /// board does not know, or one it has not let act.
    Denied,
    /// The address answered with a node key other than the one it is pinned to.
    OtherKey,
    /// The comm core refused the request with a code other than `denied`. The code is kept.
    Refused,
    /// Anything else: a broken or silent connection, a protocol fault, a transfer's own error.
    Other,
}

/// A refusal on a live session: its kind, the comm core's numeric code when Orion gave one, and
/// the text exactly as it was before this type existed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    kind: FailureKind,
    code: Option<u16>,
    text: String,
}

impl Failure {
    /// A failure of `kind` with `code` (when there is one) and `text`.
    #[must_use]
    pub fn new(kind: FailureKind, code: Option<u16>, text: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            text: text.into(),
        }
    }

    /// A failure that is not one of Orion's named kinds: its text only.
    #[must_use]
    pub fn other(text: impl Into<String>) -> Self {
        Self::new(FailureKind::Other, None, text)
    }

    /// The failure of an Orion error from a call on a session. Its text is the error's text.
    // `orion::Error` has a second variant when Orion's topics are on, so the match is refutable
    // in one build and not in another.
    #[allow(irrefutable_let_patterns)]
    #[must_use]
    pub fn of(error: &OrionError) -> Self {
        if let OrionError::Client(client) = error {
            Self::of_client(client, client.to_string())
        } else {
            Self::other(error.to_string())
        }
    }

    /// The failure of a client error, with `text` as its text (a caller that shows a shorter
    /// text than the error's own passes it here).
    #[must_use]
    pub fn of_client(error: &ClientError, text: String) -> Self {
        match error {
            ClientError::Denied(_) => Self::new(FailureKind::Denied, None, text),
            ClientError::OtherKey { .. } => Self::new(FailureKind::OtherKey, None, text),
            ClientError::Refused(core) => Self::of_core(*core, text),
            _ => Self::other(text),
        }
    }

    /// A refusal the comm core made with `core`. `denied` is its own kind; any other code is
    /// [`FailureKind::Refused`] with the code.
    #[must_use]
    pub fn of_core(core: CoreError, text: String) -> Self {
        if core == CoreError::Denied {
            Self::new(FailureKind::Denied, Some(core.code()), text)
        } else {
            Self::new(FailureKind::Refused, Some(core.code()), text)
        }
    }

    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> FailureKind {
        self.kind
    }

    /// Whether the node turned this client away (see [`FailureKind::Denied`]).
    #[must_use]
    pub const fn is_denied(&self) -> bool {
        matches!(self.kind, FailureKind::Denied)
    }

    /// The comm core's numeric code, when Orion gave one.
    #[must_use]
    pub const fn code(&self) -> Option<u16> {
        self.code
    }

    /// The text, as it was.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<String> for Failure {
    fn from(text: String) -> Self {
        Self::other(text)
    }
}

impl From<&str> for Failure {
    fn from(text: &str) -> Self {
        Self::other(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orion::Error as OrionError;

    #[test]
    fn a_denied_client_error_is_denied_whatever_its_words() {
        let failure = Failure::of(&OrionError::Client(ClientError::Denied("no")));
        assert_eq!(failure.kind(), FailureKind::Denied);
        assert_eq!(failure.code(), None);
        assert_eq!(failure.text(), "no");
    }

    #[test]
    fn the_comm_cores_denied_is_denied_with_its_code() {
        let failure = Failure::of(&OrionError::Client(ClientError::Refused(CoreError::Denied)));
        assert!(failure.is_denied());
        assert_eq!(failure.code(), Some(CoreError::Denied.code()));
    }

    #[test]
    fn another_comm_core_refusal_keeps_its_code_and_is_not_denied() {
        let failure = Failure::of(&OrionError::Client(ClientError::Refused(CoreError::Busy)));
        assert_eq!(failure.kind(), FailureKind::Refused);
        assert_eq!(failure.code(), Some(CoreError::Busy.code()));
        assert!(!failure.is_denied());
    }

    #[test]
    fn a_non_denied_error_whose_text_says_denied_is_not_denied() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let failure = Failure::of(&OrionError::Client(ClientError::Io(io)));
        assert_eq!(failure.kind(), FailureKind::Other);
        assert!(!failure.is_denied());
        assert!(failure.text().contains("denied"));
        assert!(!Failure::other("denied").is_denied());
    }

    #[test]
    fn a_key_mismatch_is_its_own_kind() {
        let failure = Failure::of(&OrionError::Client(ClientError::OtherKey {
            asked: [1; 32],
            answered: [2; 32],
        }));
        assert_eq!(failure.kind(), FailureKind::OtherKey);
    }
}
