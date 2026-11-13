//! Why an action did not give a result. Each message is the board's own words where the board
//! gave some: a refusal reads exactly as the board refused it.

use std::time::Duration;

use atlas_jobs::JobError;
use thiserror::Error;

/// Why an action call gave no result.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ActionError {
    /// The board does not list this action, so it is not sent.
    #[error("the board does not list {0}")]
    NotListed(String),
    /// The board's list is there but is not a list Atlas reads.
    #[error("the board's action list cannot be read: {0}")]
    BadCatalog(String),
    /// The arguments do not fit the board's description. Nothing was sent.
    #[error("{0}")]
    BadArguments(String),
    /// A long action, or one the board does not say is short: start it as a job.
    #[error("{0} is run as a job: it can be followed, cancelled and re-attached to")]
    NeedsJob(String),
    /// The node is not connected. Nothing was sent, so the call can be made again.
    #[error("the node is not connected, so nothing was sent: {0}")]
    NotConnected(String),
    /// The board refused the action. The message is the board's reason.
    #[error("{0}")]
    Refused(String),
    /// The action ran and failed on the board. The message is the board's reason.
    #[error("{0}")]
    Failed(String),
    /// The action was cancelled. The message is the board's reason, or what the cancel did.
    #[error("{0}")]
    Cancelled(String),
    /// A short action did not finish within its wait. It was asked to stop; a longer action runs
    /// as a job.
    #[error(
        "{name} did not finish within {wait:?}; it was asked to stop, and a longer action runs as a job"
    )]
    NotFinished { name: String, wait: Duration },
    /// A short action's call did not come back. It may have run and may still be running; the
    /// board cancels a run whose session ends. The caller decides what to do next.
    #[error("the outcome of {name} is not known: {reason}")]
    Unknown { name: String, reason: String },
    /// The start was sent but the node did not confirm it. It may have started, so it is not
    /// sent again; check the board's state before starting it on purpose.
    #[error("the board did not confirm that {name} started, so it was not sent again: {reason}")]
    StartInDoubt { name: String, reason: String },
    /// The session ended while the run was going. The run is still on the board under `call`
    /// (a detached run carries on), and [`crate::BoardActions::follow`] comes back to it.
    #[error("the connection ended while run {call} was going: {reason}")]
    Lost { call: u64, reason: String },
    /// The node did not answer in time. Nothing is known about a read; the call can be made
    /// again.
    #[error("the node did not answer: {0}")]
    NotAnswering(String),
    /// The session was silent for longer than the board's lease while this run's hold existed:
    /// the board may have dropped the hold, and with it the run. The outcome on the board is not
    /// known from here. `reason` is the board's own words when the run had already ended with an
    /// error, else `not finished`.
    #[error(
        "the board may have dropped the hold on run {call}: no request reached its bridge within the lease (the board said: {reason})"
    )]
    HoldLapsed { call: u64, reason: String },
    /// The job could not be queued (a second job holds the board, or the job list is refused).
    #[error(transparent)]
    Job(#[from] JobError),
}

impl ActionError {
    /// The message the board gave, when it gave one: the reason of a refusal, a failure or a
    /// cancel, exactly as it came.
    #[must_use]
    pub fn board_reason(&self) -> Option<&str> {
        match self {
            Self::Refused(reason) | Self::Failed(reason) | Self::Cancelled(reason) => Some(reason),
            _ => None,
        }
    }
}
