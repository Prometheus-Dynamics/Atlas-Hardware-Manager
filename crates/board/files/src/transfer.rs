//! How a push is sent on a node's session: the seam to Orion's file API.
//!
//! A push is a [`SteppedTransfer`] of the session (see `atlas_session::long`). Its adapter is the
//! only code here that names Orion's file types:
//!
//! - [`Blocking`] is the adapter for the locked Orion rev (455a956e). Its one step is Orion's
//!   blocking `Files::push`, which runs the whole file. It holds the node's session for as long as
//!   the file takes: no other call runs on that node until it ends. The structure is in place for
//!   the stepped form; the interleaving itself arrives with Orion R2.
//! - The stepped adapter (feature `orion-r2`) is written against the shape Orion announced for R2
//!   (`Files::start_push` and `Pushing::step`). It is not built at the lock, and the gate does not
//!   enable the feature.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_session::{Step, StepError, SteppedTransfer};
use orion::Orion;
use orion::files::{Files, Transfer};

use crate::source::{Guard, PushSource, Shared};

/// Starts a push of `source` to `destination` on the session's client, as a stepped transfer.
/// Runs on the session thread, inside the start closure of `Handle::long`.
pub(crate) fn start(
    orion: &Orion,
    destination: String,
    source: Arc<Mutex<Box<dyn PushSource>>>,
    shared: Arc<Shared>,
) -> Result<Box<dyn SteppedTransfer<Transfer>>, orion::Error> {
    #[cfg(feature = "orion-r2")]
    {
        stepped::start(orion.files(), destination, source, shared)
    }
    #[cfg(not(feature = "orion-r2"))]
    {
        Ok(Box::new(Blocking {
            files: orion.files().clone(),
            destination,
            source,
            shared,
            sent: false,
        }))
    }
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

/// The adapter for the locked Orion rev: one step is the whole push, through its blocking call.
// LOCK: at 455a956e `Files::push` is one blocking loop. Its only hook while it runs is the progress
// callback, which cannot return control to the session, so this adapter cannot interleave calls.
// Remove this adapter when the stepped form lands (see the `orion-r2` module below).
struct Blocking {
    files: Files,
    destination: String,
    source: Arc<Mutex<Box<dyn PushSource>>>,
    shared: Arc<Shared>,
    /// Set once the one step has run; the transfer is not stepped again after it.
    sent: bool,
}

impl SteppedTransfer<Transfer> for Blocking {
    fn step(&mut self, _wait: Duration) -> Result<Step<Transfer>, StepError> {
        if self.sent {
            return Err(StepError::Failed("the push already ran".to_owned()));
        }
        self.sent = true;
        // The step does not honour its wait: the whole file runs here (see `atlas_session::long`).
        let mut guard = Guard::new(Arc::clone(&self.source), Arc::clone(&self.shared));
        let shared = Arc::clone(&self.shared);
        self.files
            .push(&self.destination, &mut guard, |received, total| {
                shared.report(received, total);
            })
            .map(Step::Done)
            .map_err(|error| classify(orion::Error::from(error)))
    }

    fn cancel(&mut self) {
        // Nothing to stop: the push runs inside its one step. The guard reads the cancel request
        // on every read, so the push ends at its next chunk.
    }
}

// LOCK: Orion R2 stepped push (`Files::start_push` / `Pushing::step`, announced in the coordinator's
// Orion shapes for the rev after 455a956e). Not built at the lock: the `orion-r2` feature is off by
// default. When the lock moves, check the names and argument order below against the rev. The
// `Pushed` names are the announced `Progress | Done(Transfer)` shape, not yet checked.
#[cfg(feature = "orion-r2")]
mod stepped {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use atlas_session::{Step, StepError, SteppedTransfer};
    use orion::files::{Files, Transfer};

    use super::classify;
    use crate::source::{Guard, PushSource, Shared};

    pub(super) fn start(
        files: &Files,
        destination: String,
        source: Arc<Mutex<Box<dyn PushSource>>>,
        shared: Arc<Shared>,
    ) -> Result<Box<dyn SteppedTransfer<Transfer>>, orion::Error> {
        // LOCK: Orion R2 `Files::start_push`. The guard owns its handles, so the push is `'static`.
        let pushing = files.start_push(&destination, Guard::new(source, shared))?;
        Ok(Box::new(Stepped {
            pushing: Some(pushing),
        }))
    }

    struct Stepped {
        pushing: Option<orion::files::Pushing>,
    }

    impl SteppedTransfer<Transfer> for Stepped {
        fn step(&mut self, wait: Duration) -> Result<Step<Transfer>, StepError> {
            let Some(pushing) = self.pushing.as_mut() else {
                return Err(StepError::Failed("the push already ended".to_owned()));
            };
            // LOCK: Orion R2 `Pushing::step(wait)`: `Pushed::Progress` or `Pushed::Done(Transfer)`.
            match pushing.step(wait) {
                Ok(orion::files::Pushed::Progress) => Ok(Step::Progress),
                Ok(orion::files::Pushed::Done(transfer)) => {
                    self.pushing = None;
                    Ok(Step::Done(transfer))
                }
                Err(error) => Err(classify(orion::Error::from(error))),
            }
        }

        fn cancel(&mut self) {
            // Dropping the push ends it; the board keeps the bytes it holds.
            self.pushing = None;
        }
    }
}
