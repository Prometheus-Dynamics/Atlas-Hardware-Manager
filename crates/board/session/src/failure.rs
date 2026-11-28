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
    /// The comm core had no room for the request yet (`busy`, code 11, with no words of its own:
    /// Orion's `is_busy`): a bounded queue is full, and asking again later may find room. Orion's own
    /// BUSY only; a Lemnos BUSY is [`FailureKind::Lemnos`].
    Busy,
    /// The comm core refused because another holds what was asked for (code 11 too, with words that
    /// name the holder: Orion's `is_held`): a resource under another's lease, a file prefix another
    /// offers. Waiting does not clear it, so it is never asked again; the words are shown.
    Held,
    /// The board's Lemnos program refused the action with its own status code, as a rejected run
    /// (Lemnos BUSY is 16). Its code is kept. Orion's codes are never this kind.
    Lemnos,
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

    /// The failure of a client error. A refusal's text is the node's words (`detail`), never empty;
    /// the other kinds keep `text`, which the caller gives (a shorter text than the error's own).
    #[must_use]
    pub fn of_client(error: &ClientError, text: String) -> Self {
        match error {
            ClientError::Denied(_) => Self::new(FailureKind::Denied, None, text),
            ClientError::OtherKey { .. } => Self::new(FailureKind::OtherKey, None, text),
            ClientError::Refused { error, detail } => Self::of_core(*error, detail.clone()),
            _ => Self::other(text),
        }
    }

    /// A refusal the comm core made with `core` and the words `text`. `denied` is its own kind; `busy`
    /// is [`FailureKind::Busy`] when it is want of room and [`FailureKind::Held`] when another holds
    /// what was asked for; any other code is [`FailureKind::Refused`] with the code. The code is never
    /// read as a Lemnos status: Orion's `DigestMismatch` is 16 too, and it is only a refusal.
    ///
    /// Want of room and held share code 11 and are told apart as Orion tells them apart: a refusal for
    /// want of room has no words of its own, so its detail is exactly the code's text.
    // board: Orion d361b592 crates/client/connect/src/error.rs:256 (is_busy: detail == "busy"; is_held: any other detail)
    #[must_use]
    pub fn of_core(core: CoreError, text: String) -> Self {
        let kind = match core {
            CoreError::Denied => FailureKind::Denied,
            CoreError::Busy if text.is_empty() || text == CoreError::Busy.as_str() => {
                FailureKind::Busy
            }
            CoreError::Busy => FailureKind::Held,
            _ => FailureKind::Refused,
        };
        Self::new(kind, Some(core.code()), text)
    }

    /// A refusal the board's Lemnos program made with its status `code` (a rejected run's status).
    #[must_use]
    pub fn lemnos(code: u16, text: impl Into<String>) -> Self {
        Self::new(FailureKind::Lemnos, Some(code), text)
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
        let failure = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::Denied,
            "",
        )));
        assert!(failure.is_denied());
        assert_eq!(failure.code(), Some(CoreError::Denied.code()));
    }

    #[test]
    fn orions_busy_is_its_own_kind_with_its_code() {
        let failure = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::Busy,
            "",
        )));
        assert_eq!(failure.kind(), FailureKind::Busy);
        assert_eq!(failure.code(), Some(11));
        assert!(!failure.is_denied());
    }

    #[test]
    fn another_comm_core_refusal_keeps_its_code_and_is_not_denied() {
        let failure = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::DigestMismatch,
            "digest does not match",
        )));
        assert_eq!(failure.kind(), FailureKind::Refused);
        assert_eq!(failure.code(), Some(16));
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

    #[test]
    fn a_refusal_takes_its_kind_from_its_code_and_its_text_from_the_nodes_words() {
        let detail = "no USB link named sda here; known: sdb, sdc";
        let failure = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::Denied,
            detail,
        )));
        assert_eq!(failure.kind(), FailureKind::Denied);
        assert_eq!(failure.code(), Some(CoreError::Denied.code()));
        assert_eq!(failure.text(), detail);
    }

    #[test]
    fn the_lease_lost_code_keeps_its_code_and_the_nodes_words_verbatim() {
        let lost = CoreError::from_code(22).expect("code 22 is lease lost");
        assert_eq!(lost, CoreError::LeaseLost);
        let detail = "lease lost while quiet: taken by atlas-next";
        let failure = Failure::of(&OrionError::Client(ClientError::refused(lost, detail)));
        assert_eq!(failure.kind(), FailureKind::Refused);
        assert_eq!(failure.code(), Some(22));
        assert_eq!(failure.text(), detail);
    }

    #[test]
    fn a_lease_held_by_another_is_denied_and_says_who_holds_it() {
        let detail = "cam0 is held by atlas-next: only its holder or root frees it";
        let failure = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::Denied,
            detail,
        )));
        assert!(failure.is_denied());
        assert_eq!(failure.text(), detail);
    }

    #[test]
    fn busy_with_the_holders_words_is_held_and_want_of_room_is_busy() {
        // Orion sends a want-of-room refusal with the code's own text and nothing more.
        let room = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::Busy,
            "busy",
        )));
        assert_eq!(room.kind(), FailureKind::Busy);
        let held = Failure::of(&OrionError::Client(ClientError::refused(
            CoreError::Busy,
            "slots is held by operator:bench since 1200 ms: an install",
        )));
        assert_eq!(held.kind(), FailureKind::Held);
        assert_eq!(held.code(), Some(CoreError::Busy.code()));
        assert_eq!(
            held.text(),
            "slots is held by operator:bench since 1200 ms: an install"
        );
    }
}
