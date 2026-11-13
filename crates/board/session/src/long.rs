//! Long operations: a transfer that is advanced in steps on the session's own thread, between the
//! session's calls, so a file push does not hold the node for its whole length.
//!
//! A long operation is started like a call (on the live client, by [`crate::Handle::long`]) and
//! returns a [`SteppedTransfer`]. The session then advances it one [`SteppedTransfer::step`] at a
//! time. Between steps the session runs the calls that are queued, then takes one step of every
//! other operation, in turn. The trait names nothing of Orion: the loop never sees Orion's types,
//! and each adapter (see the `atlas-files` crate) keeps its own client handle inside the transfer.
//!
//! # Fairness
//!
//! One turn of the session is: the calls queued when the turn starts, each run once in arrival
//! order; then one step of each running operation, in start order, each given at most
//! [`STEP_WAIT`]. When operations run, the frames already waiting on the node are taken as well,
//! at most `MAX_FRAMES_PER_TURN` (64) a turn, so a session that broke is noticed between steps.
//! At most [`MAX_LONG_OPERATIONS`] operations run at once; a further one waits until a slot frees,
//! and is not stepped meanwhile.
//!
//! A short call that arrives during a turn's steps is run by the next turn. So the worst case for
//! a short call is two turns of steps, `2 * MAX_LONG_OPERATIONS * STEP_WAIT` (400 ms), plus the
//! short calls queued ahead of it. That bound holds only for an adapter that honours the `wait` it
//! is given. An adapter that cannot (today's blocking push is one step that runs the whole file)
//! holds the session for its whole length, and the bound does not apply to it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The most time one step may spend waiting on the node before it returns with
/// [`Step::Progress`]. A step is given this as its `wait`. Short enough that a short call waits
/// at most a few steps (see the module docs); long enough that a step is not a busy loop of
/// one-frame pumps.
pub const STEP_WAIT: Duration = Duration::from_millis(50);

/// How many long operations one session runs at once. Each one adds [`STEP_WAIT`] to the wait of
/// a short call, so the count is kept small: two pushes and a fetch fit with room to spare. A
/// further operation is parked (not stepped, not refused) until a slot frees.
pub const MAX_LONG_OPERATIONS: usize = 4;

/// One step of a long operation: still going, or done with its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step<T> {
    /// Not finished; the session will step it again after its other work.
    Progress,
    /// Finished, with what it produced.
    Done(T),
}

/// Why a step could not be taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepError {
    /// The session is gone. The operation ends the way a call does when the session breaks: its
    /// caller is told the node is gone, and resumes it on a later session if it means to.
    Lost(String),
    /// The operation failed for good on a live session. Retrying gives the same answer.
    Failed(String),
}

/// A transfer advanced in steps on the session thread. Created there by the start closure of
/// [`crate::Handle::long`], used only there, and dropped there. It need not be `Send`.
pub trait SteppedTransfer<T> {
    /// Advances the transfer by at most `wait`, which the session passes as [`STEP_WAIT`].
    ///
    /// # Errors
    ///
    /// [`StepError::Lost`] when the session broke during the step, [`StepError::Failed`] when the
    /// transfer failed for good on a live session. After either the transfer is not stepped again.
    fn step(&mut self, wait: Duration) -> Result<Step<T>, StepError>;

    /// Stops the transfer. Called once: when it is cancelled, when its caller stops waiting, or
    /// when its session ends. It must not block. Bytes it already moved stay where they are.
    fn cancel(&mut self);
}

/// How a long operation ended, when the session did not lose it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    /// The transfer finished with this value.
    Done(T),
    /// It was cancelled at a step boundary (or before its first step).
    Cancelled,
}

/// A request to stop a long operation. Clones share the request. The operation sees it at its
/// next step boundary, not in the middle of a step.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A request that nobody has made yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the operation to stop at its next step boundary.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether the request was made.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
