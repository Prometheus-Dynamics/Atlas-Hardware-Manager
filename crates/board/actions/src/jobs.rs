//! Long actions as jobs: progress, cancel, and re-attach after a lost session.
//!
//! Each action is one step of one job. The step is saved with the job and carries the action's
//! name, its words, the node's address and a nonce that is unique per submit, so two jobs never
//! share the state of a run. The run's number on the board is kept in memory, per nonce.
//!
//! The retry rule, which is the one the brief names: **an action is never issued twice because
//! of a retry.** `jobs` retries a step when it fails without a settled answer, so the step does
//! this:
//!
//! - Has a run number already: it follows that run (a retry is a read). It never starts again.
//! - Has none: it starts the run once. If the node is not connected, nothing was sent, and the
//!   step fails without a settled answer so `jobs` retries it. If the start was sent and the
//!   node did not confirm it, the step ends the job as failed (settled) and says it was not sent
//!   again. A job never ends by starting the run a second time.
//!
//! Cancel: `jobs` cancels a job by dropping its step, which tells the board nothing. So the
//! board is told by [`ActionJobs::cancel`], which cancels the job first and then the run, and
//! checks that the run ended cancelled. Dropping a step (a step timeout, or a cancel that came
//! another way) never cancels the run: it stays on the board and the next attempt follows it.
//!
//! Resume after an Atlas restart: the run number is not saved, so a job that was running when
//! Atlas stopped is marked interrupted (not resumable). The run is detached and keeps going on
//! the board. Following it after a restart needs the number kept on disk: an open item.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::DriverError;
use atlas_jobs::{DeviceClaim, DeviceHolds, Events, JobId, JobKind, JobRecord, JobTiming, Jobs};
use atlas_session::Sessions;
use atlas_types::DeviceKey;
use orion::Address;
use serde::{Deserialize, Serialize};
use tokio::time::{self, timeout};

use crate::args;
use crate::board::{BoardActions, Progress};
use crate::catalog::Action;
use crate::error::ActionError;

/// The name the job kind is saved under.
pub const KIND: &str = "board-action";
/// How long a cancel waits for the board to say the run ended.
pub const CANCEL_CONFIRM: Duration = Duration::from_secs(5);

/// Whether a cancel reached the run and ended it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CancelState {
    /// The run ended cancelled.
    Confirmed,
    /// The action never started on the board, so there was nothing to stop.
    NoRun,
    /// The run did not end cancelled, or the cancel did not reach the board. The reason says
    /// which; the run may still be going on the board.
    NotConfirmed(String),
}

/// One saved step: everything needed to start or follow the action.
#[derive(Serialize, Deserialize)]
struct Step {
    nonce: String,
    address: String,
    name: String,
    words: Vec<String>,
}

/// What the job kind keeps for a step that has not finished.
#[derive(Debug, Default)]
struct Tracked {
    call: Option<u64>,
    /// The start is being sent, and its answer (the run number) is not known yet.
    starting: bool,
    progress: Option<Progress>,
    cancel: Option<CancelState>,
}

/// The job kind: one board action per step.
#[derive(Debug)]
pub(crate) struct ActionKind {
    sessions: Arc<Sessions>,
    tracked: Mutex<BTreeMap<String, Tracked>>,
}

fn next_nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

impl ActionKind {
    fn new(sessions: Arc<Sessions>) -> Self {
        Self {
            sessions,
            tracked: Mutex::new(BTreeMap::new()),
        }
    }

    fn map(&self) -> MutexGuard<'_, BTreeMap<String, Tracked>> {
        self.tracked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, nonce: &str, change: impl FnOnce(&mut Tracked)) {
        change(self.map().entry(nonce.to_owned()).or_default());
    }

    fn forget(&self, nonce: &str) {
        self.map().remove(nonce);
    }

    fn call_of(&self, nonce: &str) -> Option<u64> {
        self.map().get(nonce).and_then(|tracked| tracked.call)
    }

    fn is_starting(&self, nonce: &str) -> bool {
        self.map()
            .get(nonce)
            .is_some_and(|tracked| tracked.starting)
    }

    fn progress_of(&self, nonce: &str) -> Option<Progress> {
        self.map()
            .get(nonce)
            .and_then(|tracked| tracked.progress.clone())
    }

    fn cancel_of(&self, nonce: &str) -> Option<CancelState> {
        self.map()
            .get(nonce)
            .and_then(|tracked| tracked.cancel.clone())
    }
}

#[async_trait]
impl JobKind for ActionKind {
    fn name(&self) -> &str {
        KIND
    }

    fn resumable(&self) -> bool {
        false
    }

    async fn run_step(&self, _claim: &DeviceClaim, step: &str) -> Result<(), DriverError> {
        let spec: Step = serde_json::from_str(step).map_err(|error| {
            DriverError::Unsupported(format!("a saved action step is not one: {error}"))
        })?;
        let address = Address::from_str(&spec.address).map_err(|error| {
            DriverError::Unsupported(format!("a saved action has no node address: {error}"))
        })?;
        let Some(handle) = self.sessions.handle(&address) else {
            return Err(DriverError::Unreachable(format!(
                "{address} is turned off, so {} is not being followed",
                spec.name
            )));
        };
        let board = BoardActions::new(handle);

        let known = self.map().get(&spec.nonce).and_then(|tracked| tracked.call);
        let call = match known {
            // A run that is already on the board is followed, never started again.
            Some(call) => call,
            None => {
                self.update(&spec.nonce, |tracked| tracked.starting = true);
                let started = board
                    .start_encoded(&spec.name, args::encode(&spec.words))
                    .await;
                self.update(&spec.nonce, |tracked| tracked.starting = false);
                match started {
                    Ok(call) => {
                        self.update(&spec.nonce, |tracked| tracked.call = Some(call));
                        call
                    }
                    // Nothing was sent: a retry is safe.
                    Err(ActionError::NotConnected(reason)) => {
                        return Err(DriverError::Unreachable(reason));
                    }
                    // The start may have reached the board. It is never sent again.
                    Err(error) => {
                        self.forget(&spec.nonce);
                        return Err(DriverError::Incompatible(error.to_string()));
                    }
                }
            }
        };

        let nonce = spec.nonce.clone();
        let followed = board
            .follow(call, |progress| {
                self.update(&nonce, |tracked| tracked.progress = Some(progress.clone()));
            })
            .await;
        match followed {
            Ok(_) => {
                self.forget(&spec.nonce);
                Ok(())
            }
            // The run is still on the board: the next attempt follows it.
            Err(error @ ActionError::Lost { .. }) => {
                Err(DriverError::Unreachable(error.to_string()))
            }
            // The board's own answer: the job ends with its words.
            Err(error) => {
                self.forget(&spec.nonce);
                Err(DriverError::Incompatible(error.to_string()))
            }
        }
    }
}

/// The jobs that run board actions: submitted, followed, cancelled, saved and resumed.
#[derive(Debug)]
pub struct ActionJobs {
    jobs: Jobs,
    kind: Arc<ActionKind>,
}

impl ActionJobs {
    /// Opens the job list at `path` with the board-action kind registered. `sessions` is the
    /// one session registry of the program: every node a job names is reached through it.
    ///
    /// # Errors
    ///
    /// As [`Jobs::open`]: a list that cannot be read or is another format.
    pub async fn open(
        path: impl Into<PathBuf>,
        sessions: Arc<Sessions>,
        events: Events,
        timing: JobTiming,
        holds: DeviceHolds,
    ) -> io::Result<Self> {
        let kind = Arc::new(ActionKind::new(sessions));
        let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone()];
        let jobs = Jobs::open(path, kinds, events, timing, holds).await?;
        Ok(Self { jobs, kind })
    }

    /// The job list, for listing and events.
    #[must_use]
    pub const fn jobs(&self) -> &Jobs {
        &self.jobs
    }

    /// Queues `action` with `words` on the node at `address`, as a job that writes to `device`.
    /// The words are checked first. The job holds the device until it ends.
    ///
    /// # Errors
    ///
    /// [`ActionError::BadArguments`] (nothing queued), or [`ActionError::Job`] when the job list
    /// refuses it (the device is held by another job, for one).
    pub async fn submit(
        &self,
        device: DeviceKey,
        address: &Address,
        action: &Action,
        words: &[String],
    ) -> Result<JobId, ActionError> {
        args::check(action, words)?;
        let step = Step {
            nonce: next_nonce(),
            address: address.to_string(),
            name: action.name.clone(),
            words: words.to_vec(),
        };
        let text = serde_json::to_string(&step)
            .map_err(|error| ActionError::BadArguments(error.to_string()))?;
        Ok(self
            .jobs
            .submit(KIND, DeviceClaim::writes([device]), vec![text])
            .await?)
    }

    /// How far the job's run has got, as the board last reported it. `None` once the job has
    /// ended or before the board has reported anything.
    #[must_use]
    pub fn progress(&self, id: JobId) -> Option<Progress> {
        let nonce = self.nonce_of(id)?;
        self.kind.progress_of(&nonce)
    }

    /// Whether a cancel reached the job's run, once one has been asked for. `None` until then.
    #[must_use]
    pub fn cancel_state(&self, id: JobId) -> Option<CancelState> {
        let nonce = self.nonce_of(id)?;
        self.kind.cancel_of(&nonce)
    }

    /// Cancels the job, then asks the board to stop its run, and checks that the run ended
    /// cancelled. Returns false if the job is not active.
    ///
    /// A start that is in flight is let finish first (within [`CANCEL_CONFIRM`]), so its run
    /// number is known and the board is asked. If the start does not finish in that time, the
    /// cancel state says the run may still start.
    pub async fn cancel(&self, id: JobId) -> bool {
        let Some(record) = self.jobs.get(id) else {
            return false;
        };
        let Some((nonce, step)) = parse_first(&record) else {
            return false;
        };
        let settled = timeout(CANCEL_CONFIRM, async {
            while self.kind.is_starting(&nonce) {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok();
        let call = self.kind.call_of(&nonce);
        if !self.jobs.cancel(id) {
            return false;
        }
        let state = match call {
            Some(call) => self.stop_on_board(&step.address, call).await,
            None if settled => CancelState::NoRun,
            None => CancelState::NotConfirmed(
                "the start did not answer in time, so the run may still start on the board".into(),
            ),
        };
        self.kind
            .update(&nonce, |tracked| tracked.cancel = Some(state));
        true
    }

    async fn stop_on_board(&self, address: &str, call: u64) -> CancelState {
        let Ok(address) = Address::from_str(address) else {
            return CancelState::NotConfirmed(format!("{address} is not an address"));
        };
        let Some(handle) = self.kind.sessions.handle(&address) else {
            return CancelState::NotConfirmed(format!(
                "{address} is turned off; run {call} may still be going"
            ));
        };
        let board = BoardActions::new(handle);
        if let Err(error) = board.cancel(call).await {
            return CancelState::NotConfirmed(format!(
                "the cancel did not reach the board ({error}); run {call} may still be going"
            ));
        }
        match timeout(CANCEL_CONFIRM, board.follow(call, |_| {})).await {
            Ok(Err(ActionError::Cancelled(_))) => CancelState::Confirmed,
            Ok(Ok(_)) => {
                CancelState::NotConfirmed("the run finished before the cancel took effect".into())
            }
            Ok(Err(other)) => CancelState::NotConfirmed(other.to_string()),
            Err(_) => CancelState::NotConfirmed(format!(
                "the board did not confirm that run {call} stopped"
            )),
        }
    }

    fn nonce_of(&self, id: JobId) -> Option<String> {
        parse_first(&self.jobs.get(id)?).map(|(nonce, _)| nonce)
    }
}

fn parse_first(record: &JobRecord) -> Option<(String, Step)> {
    let step: Step = serde_json::from_str(record.steps.first()?).ok()?;
    Some((step.nonce.clone(), step))
}
