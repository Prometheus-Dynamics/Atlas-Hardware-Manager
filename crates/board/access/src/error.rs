//! Why a terminal or a key install did not happen.

use std::fmt;

/// Why an access call gave no result. None of these ends a session by itself: a terminal that
/// is open ends with its own [`End`](crate::End).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessError {
    /// The node is not connected: the session says why. Retry later, by a person.
    Gone(String),
    /// The node's hello does not say whether it runs or is in recovery, so no shell kind is
    /// chosen for it. Nothing is guessed.
    NoState,
    /// The node says a state this crate does not know.
    UnknownState(String),
    /// The node's key is not the one the caller knew this board by. The session's rule: a
    /// changed key is never accepted silently.
    KeyChanged {
        /// The fingerprint the node now shows, for people to compare.
        found: String,
    },
    /// A key line is not a public key this crate accepts. Refused before anything is sent.
    BadKey(&'static str),
    /// A terminal request that cannot be made: a size of zero, or a terminal name that is not
    /// plain.
    BadRequest(&'static str),
    /// The carrier or the node refused the call, with its words.
    Refused(String),
    /// The shell stream failed, with its words. The terminal is already ended with this.
    Stream(String),
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone(reason) => write!(f, "the node is not connected: {reason}"),
            Self::NoState => write!(
                f,
                "the node does not say whether it is running or in recovery"
            ),
            Self::UnknownState(word) => write!(f, "the node says an unknown state: {word}"),
            Self::KeyChanged { found } => write!(
                f,
                "the node's key has changed (it now shows {found}); nothing was opened"
            ),
            Self::BadKey(why) => write!(f, "not a key that can be installed: {why}"),
            Self::BadRequest(why) => write!(f, "the terminal request is not valid: {why}"),
            Self::Refused(words) => write!(f, "refused: {words}"),
            Self::Stream(words) => write!(f, "the shell stream failed: {words}"),
        }
    }
}

impl std::error::Error for AccessError {}
