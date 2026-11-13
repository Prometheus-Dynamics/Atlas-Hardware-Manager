//! The stages of a slot install, as the app shows them.
//!
//! An install goes through these in order: making the bundle on the computer, waiting for the
//! board, sending the bundle, the board installing it, and done. A failure or a cancel ends it.
//! A lost session puts it back to [`Stage::WaitingForBoard`] until the session is back; it does not
//! end the install.

use std::fmt;

/// Where an install is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Making the slot bundle from the image on the computer.
    Preparing,
    /// The board is not reachable, or the sender is waiting for it. `reason` says why.
    WaitingForBoard { reason: String },
    /// The bundle is being sent: bytes the board has taken of the total.
    Sending { sent: u64, total: u64 },
    /// The board is installing: percent done (when it says) and its own words.
    Installing { percent: Option<u8>, message: String },
    /// Done: the new slot's id.
    Done { slot: String },
    /// Ended without a slot. `reason` is the board's words when it refused, or Atlas's.
    Failed { reason: String },
    /// Cancelled by the person.
    Cancelled,
}

impl Stage {
    /// Whether the install has ended (done, failed or cancelled).
    #[must_use]
    pub const fn is_final(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed { .. } | Self::Cancelled)
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparing => f.write_str("preparing the bundle"),
            Self::WaitingForBoard { reason } => write!(f, "waiting for the board: {reason}"),
            Self::Sending { sent, total } => write!(f, "sending {sent} of {total} bytes"),
            Self::Installing { percent: Some(percent), message } => {
                write!(f, "the board is installing, {percent}%: {message}")
            }
            Self::Installing { percent: None, message } => {
                write!(f, "the board is installing: {message}")
            }
            Self::Done { slot } => write!(f, "installed as {slot}"),
            Self::Failed { reason } => write!(f, "failed: {reason}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}
