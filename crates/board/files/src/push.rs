//! A push as a job: the file goes to the node's session in one job step, which waits through a
//! lost session and goes on by itself.
//!
//! Each push is one step of one [`atlas_jobs`] job of [`PushKind`]. The step runs Orion's
//! `Files::push` on the node's live session, through [`atlas_session::Handle::with_client`]. When the
//! session breaks, the step waits for it and begins the file again: the board answers with how
//! many bytes it holds, and the push carries on from there, so a drop costs at most the window
//! Orion keeps in flight. The step ends only when the board commits the file, the source changes
//! or cannot be read, the board refuses the file, or a person cancels. A session that never comes
//! back is waited for, never given up on.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::DriverError;
use atlas_jobs::{DeviceClaim, JobError, JobId, JobKind, Jobs};
use atlas_session::{Backoff, Handle, Presence};
use orion::files::FileState;
use orion::types::FileName;
use tokio::sync::watch;

use crate::progress::{Progress, Stage};
use crate::source::{Counters, Guard, PushSource, Shared};

/// The job kind's name, as saved with each push.
pub const KIND: &str = "file-push";

/// How long one push call may run before it is given up as a node that does not answer. A push
/// is long by nature: this bounds only a call that never returns, since a session that stops
/// answering ends the call first. Pass it through [`atlas_session::Handle::waiting`].
pub const PUSH_WAIT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The job kind for pushes. Register it once with the jobs: `Jobs::open` takes it as an
/// `Arc<dyn JobKind>`, and [`submit`] needs the same instance.
#[derive(Default)]
pub struct PushKind {
    pending: Mutex<BTreeMap<String, Arc<Pending>>>,
    next: AtomicU64,
}

impl fmt::Debug for PushKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushKind").finish_non_exhaustive()
    }
}

impl PushKind {
    /// A new kind, ready to register with the jobs.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Arc<Pending>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn forget(&self, step: &str) {
        self.lock().remove(step);
    }
}

/// What one push needs to run, kept until the push ends.
struct Pending {
    handle: Handle,
    destination: String,
    source: Arc<Mutex<Box<dyn PushSource>>>,
    shared: Arc<Shared>,
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pending")
            .field("destination", &self.destination)
            .field("shared", &self.shared)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl JobKind for PushKind {
    fn name(&self) -> &str {
        KIND
    }

    /// A push is not resumed by a restart of Atlas: its job is marked interrupted, and pushing
    /// the file again resumes it on the board, since the board keeps what it holds.
    fn resumable(&self) -> bool {
        false
    }

    async fn run_step(&self, _claim: &DeviceClaim, step: &str) -> Result<(), DriverError> {
        let pending = self.lock().get(step).cloned().ok_or_else(|| {
            DriverError::Incompatible(
                "this push's source is no longer held; push the file again".to_owned(),
            )
        })?;
        let answer = run(&pending).await;
        self.forget(step);
        answer.map_err(|reason| {
            pending.shared.finish(Stage::Failed {
                reason: reason.clone(),
            });
            DriverError::Incompatible(reason)
        })
    }
}

/// Why a push could not be started.
#[derive(Debug)]
pub enum PushError {
    /// The destination is not a file name the board accepts: no empty names, no `..`, no
    /// characters outside the board's set.
    Destination(String),
    /// The source could not be measured: its length, or its start, cannot be read.
    Source(io::Error),
    /// The jobs refused the push.
    Job(JobError),
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Destination(name) => write!(f, "`{name}` is not a file name a board accepts"),
            Self::Source(error) => write!(f, "the source cannot be read: {error}"),
            Self::Job(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PushError {}

/// A push that has started: its job, its progress, and the way to cancel it.
pub struct Push {
    job: JobId,
    jobs: Jobs,
    kind: Arc<PushKind>,
    step: String,
    shared: Arc<Shared>,
    progress: watch::Receiver<Progress>,
}

impl fmt::Debug for Push {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Push")
            .field("job", &self.job)
            .field("progress", &self.progress())
            .finish_non_exhaustive()
    }
}

impl Push {
    /// The job this push runs as. Its status is in [`Jobs::get`]: running while it waits for the
    /// board, done when the board holds the file, failed or cancelled otherwise.
    #[must_use]
    pub const fn job(&self) -> JobId {
        self.job
    }

    /// Where the push is now.
    #[must_use]
    pub fn progress(&self) -> Progress {
        self.progress.borrow().clone()
    }

    /// A receiver of progress changes, for code that waits on them.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<Progress> {
        self.progress.clone()
    }

    /// What the push has done over all its attempts: how many times it began on a session, and
    /// how many bytes it read to send.
    #[must_use]
    pub fn counters(&self) -> Counters {
        self.shared.counters()
    }

    /// Stops the push. Bytes the board already holds stay there, and pushing the same file again
    /// resumes from them. Returns false when the push had already ended.
    pub fn cancel(&self) -> bool {
        self.shared.cancel();
        let stopped = self.jobs.cancel(self.job);
        if stopped {
            self.shared.finish(Stage::Cancelled);
        }
        self.kind.forget(&self.step);
        stopped
    }
}

/// Starts a push of `source` to `destination` on the node behind `handle`, as a job of `kind`.
///
/// The destination is the name the file has on the board. The source is read from its start; it
/// must not change while the push runs. A change is detected and refused, not sent.
///
/// # Errors
///
/// If the destination is not a name the board accepts, the source cannot be measured, or the jobs
/// refuse it.
pub async fn submit(
    kind: &Arc<PushKind>,
    jobs: &Jobs,
    handle: &Handle,
    destination: &str,
    mut source: Box<dyn PushSource>,
) -> Result<Push, PushError> {
    FileName::new(destination).map_err(|_| PushError::Destination(destination.to_owned()))?;
    let total = source.seek(SeekFrom::End(0)).map_err(PushError::Source)?;
    source.seek(SeekFrom::Start(0)).map_err(PushError::Source)?;

    let (shared, progress) = Shared::new(total);
    let shared = Arc::new(shared);
    let number = kind.next.fetch_add(1, Ordering::SeqCst) + 1;
    let step = format!("push-{number}");
    let pending = Arc::new(Pending {
        handle: handle.waiting(PUSH_WAIT),
        destination: destination.to_owned(),
        source: Arc::new(Mutex::new(source)),
        shared: Arc::clone(&shared),
    });
    kind.lock().insert(step.clone(), pending);
    match jobs
        .submit(KIND, DeviceClaim::none(), vec![step.clone()])
        .await
    {
        Ok(job) => Ok(Push {
            job,
            jobs: jobs.clone(),
            kind: Arc::clone(kind),
            step,
            shared,
            progress,
        }),
        Err(error) => {
            kind.forget(&step);
            Err(PushError::Job(error))
        }
    }
}

/// The push itself: attempts until the board commits, a refusal ends it, or it is cancelled.
/// `Err` is the reason it stopped for good.
async fn run(pending: &Pending) -> Result<(), String> {
    let shared = &pending.shared;
    let mut backoff = Backoff::default();
    loop {
        if shared.cancelled() {
            return Err("the push was cancelled".to_owned());
        }
        if let Some(reason) = shared.refusal() {
            return Err(reason);
        }
        let presence = pending.handle.presence();
        if !matches!(presence, Presence::Found(_)) {
            shared.waiting(presence_reason(&presence));
            wait_for_board(&pending.handle, backoff.next_delay()).await;
            continue;
        }

        let before = shared.acked();
        shared.sending();
        let attempt = Arc::clone(shared);
        let source = Arc::clone(&pending.source);
        let destination = pending.destination.clone();
        let answer = pending
            .handle
            .with_client(move |orion| {
                let mut source = source.lock().unwrap_or_else(PoisonError::into_inner);
                let mut guard = Guard::new(&mut **source, &attempt);
                orion
                    .files()
                    .push(&destination, &mut guard, |received, total| {
                        attempt.report(received, total);
                    })
                    .map_err(orion::Error::from)
            })
            .await;

        match answer {
            Ok(transfer) => match transfer.state {
                FileState::Committed => {
                    let total = shared.current().total;
                    if transfer.received != total {
                        return Err(format!(
                            "the board committed {} of {total} bytes",
                            transfer.received
                        ));
                    }
                    shared.finish(Stage::Committed);
                    return Ok(());
                }
                FileState::Rejected => {
                    return Err(format!("the board rejected the file: {}", transfer.detail));
                }
                // The board is still checking the file, or answers in a state this build does not
                // know: ask again, as for a lost session.
                _ => {
                    shared.waiting("the board is checking the file".to_owned());
                    tokio::time::sleep(backoff.next_delay()).await;
                }
            },
            Err(atlas_session::SessionError::Gone(reason)) => {
                if let Some(reason) = shared.refusal() {
                    return Err(reason);
                }
                if shared.cancelled() {
                    return Err("the push was cancelled".to_owned());
                }
                if shared.acked() > before {
                    backoff.reset();
                }
                shared.waiting(reason);
                tokio::time::sleep(backoff.next_delay()).await;
            }
            Err(atlas_session::SessionError::Failed(reason)) => {
                return Err(shared.refusal().unwrap_or(reason));
            }
        }
    }
}

fn presence_reason(presence: &Presence) -> String {
    match presence {
        Presence::Lost(reason) => reason.clone(),
        _ => "the node is not connected yet".to_owned(),
    }
}

/// Waits until the node is connected again, or `delay` has passed: the wait is never longer than
/// the failure rule's cap, and a connection that comes back ends it at once.
async fn wait_for_board(handle: &Handle, delay: Duration) {
    let mut presence = handle.watch();
    let _ = tokio::time::timeout(
        delay,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await;
}
