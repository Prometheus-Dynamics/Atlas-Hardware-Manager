//! How a push is sent on a node's session: the seam to Orion's file API.
//!
//! A push is a [`SteppedTransfer`] of the session (see `atlas_session::long`). Its adapter is the
//! only code here that names Orion's file types. It is Orion's stepped push (`Files::start_push`
//! and `Pushing::step`, Orion R2): each step sends what the window has room for and takes what
//! the board said, so the session's other calls run between steps. A short call issued during a
//! push answers without waiting for the push to end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_session::{AnswerBound, Failure, Step, StepError, SteppedTransfer};
use orion::files::{Files, Pushing, Step as PushStep, Transfer};
use orion::{ClientError, Orion};

use crate::bounds::{BOARD_COMMIT_CEILING, ceiling_reason};
use crate::source::{Guard, PushSource, Shared};

/// Starts a push of `source` to `destination` on the session's client, as a stepped transfer.
/// Runs on the session thread, inside the start closure of `Handle::long`.
pub(crate) fn start(
    orion: &Orion,
    destination: String,
    source: Arc<Mutex<Box<dyn PushSource>>>,
    shared: Arc<Shared>,
) -> Result<Box<dyn SteppedTransfer<Transfer>>, orion::Error> {
    stepped(orion.files(), &destination, source, shared)
}

/// Sorts an Orion error the way a call is sorted: a lost session ends the session's turns, any
/// other failure is the transfer's own.
fn classify(error: orion::Error) -> StepError {
    if error.is_lost() {
        StepError::Lost(error.to_string())
    } else {
        StepError::Failed(atlas_session::Failure::of(&error))
    }
}

/// Begins the push with Orion's `start_push`. The guard owns its handles, so the push is `'static`.
/// The file's digest is not known yet, so Orion reads the source once to hash it before the first
/// chunk is sent (see the `Known` note in `source.rs`).
fn stepped(
    files: &Files,
    destination: &str,
    source: Arc<Mutex<Box<dyn PushSource>>>,
    shared: Arc<Shared>,
) -> Result<Box<dyn SteppedTransfer<Transfer>>, orion::Error> {
    let pushing = files
        .start_push(destination, Guard::new(source, Arc::clone(&shared)), None)
        .map_err(orion::Error::from)?;
    Ok(Box::new(Stepped {
        pushing: Some(pushing),
        shared,
        // Told before the first step; until then, the ceiling this push asks for.
        bound: AnswerBound::Alive {
            ceiling: BOARD_COMMIT_CEILING,
        },
    }))
}

/// One push on the session: the Orion push while it runs, `None` once it has ended.
struct Stepped {
    pushing: Option<Pushing<Guard>>,
    shared: Arc<Shared>,
    /// The bound the session set for this step's answers (see `answer_bound`).
    bound: AnswerBound,
}

impl SteppedTransfer<Transfer> for Stepped {
    fn answer_bound(&mut self, bound: AnswerBound) {
        self.bound = bound;
    }

    fn reply_ceiling(&self) -> Option<Duration> {
        // The board's commit (its check of the whole file) is the one answer that takes long; the
        // session waits for it as long as the board is heard from, up to the ceiling.
        Some(BOARD_COMMIT_CEILING)
    }

    fn step(&mut self, wait: Duration) -> Result<Step<Transfer>, StepError> {
        let Some(pushing) = self.pushing.as_mut() else {
            return Err(StepError::Failed("the push already ended".into()));
        };
        match pushing.step(wait) {
            // `Waiting` is a step in which the board has not yet said how much it has.
            Ok(PushStep::Waiting) => Ok(Step::Progress),
            Ok(PushStep::Progress { received, size }) => {
                self.shared.report(received, size);
                Ok(Step::Progress)
            }
            // Orion d4010a45: a push the board has given nothing more of for a while. Orion keeps
            // waiting and says so; so does the transfer (never given up on unless asked).
            Ok(PushStep::Stalled { received, size, .. }) => {
                if let Some(size) = size {
                    self.shared.report(received, size);
                }
                Ok(Step::Progress)
            }
            Ok(PushStep::Done(transfer)) => {
                self.pushing = None;
                Ok(Step::Done(transfer))
            }
            Err(ClientError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut => {
                // A timed-out wait is one answer that took longer than the bound the session set
                // for this step (see `answer_bound`): under the ceiling, the board still answering
                // past it. A refusal of the push, not a lost session, so the session is kept and the
                // words name the bound that ran out.
                self.pushing = None;
                let (sent, total) = {
                    let progress = self.shared.current();
                    (progress.sent, progress.total)
                };
                Err(StepError::Failed(Failure::other(ceiling_reason(
                    sent, total, self.bound,
                ))))
            }
            Err(error) => {
                self.pushing = None;
                Err(classify(orion::Error::from(error)))
            }
        }
    }

    fn cancel(&mut self) {
        // Dropping the push ends it; the board keeps the bytes it holds for a push of the same
        // file to the same place to carry on from.
        self.pushing = None;
    }
}
