//! How a push is sent on a node's session: the seam to Orion's file API.
//!
//! A push is a [`SteppedTransfer`] of the session (see `atlas_session::long`). Its adapter is the
//! only code here that names Orion's file types. It is Orion's stepped push (`Files::start_push`
//! and `Pushing::step`, Orion R2): each step sends what the window has room for and takes what
//! the board said, so the session's other calls run between steps. A short call issued during a
//! push answers without waiting for the push to end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_session::{Step, StepError, SteppedTransfer};
use orion::Orion;
use orion::files::{Files, Pushing, Step as PushStep, Transfer};

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
        StepError::Failed(error.to_string())
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
    }))
}

/// One push on the session: the Orion push while it runs, `None` once it has ended.
struct Stepped {
    pushing: Option<Pushing<Guard>>,
    shared: Arc<Shared>,
}

impl SteppedTransfer<Transfer> for Stepped {
    fn step(&mut self, wait: Duration) -> Result<Step<Transfer>, StepError> {
        let Some(pushing) = self.pushing.as_mut() else {
            return Err(StepError::Failed("the push already ended".to_owned()));
        };
        match pushing.step(wait) {
            // `Waiting` is a step in which the board has not yet said how much it has.
            Ok(PushStep::Waiting) => Ok(Step::Progress),
            Ok(PushStep::Progress { received, size }) => {
                self.shared.report(received, size);
                Ok(Step::Progress)
            }
            Ok(PushStep::Done(transfer)) => {
                self.pushing = None;
                Ok(Step::Done(transfer))
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
