//! Jobs: ordered steps of work on one device (or none), run in the background
//! and saved after every change, so a job outlives an Atlas restart where its
//! kind can continue. Each job is its own task, so a job that waits on a
//! device never holds up another job.
//!
//! Resume rule: a job's completed steps are never run again. A step that was
//! running when Atlas stopped runs again from its start, so a resumable kind's
//! steps must be safe to repeat. A job whose kind cannot continue after a
//! restart is marked interrupted, not silently dropped, and nothing is resumed.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::DriverError;
use atlas_types::DeviceKey;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;
use tokio::time;

use crate::call::guarded;
use crate::{Backoff, Event, Events};

/// The file format version. A file from another version is refused, not guessed at.
const FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

/// A kind of work. The host registers each kind once at startup; a saved job
/// finds its kind by [`name`](Self::name) after a restart.
#[async_trait]
pub trait JobKind: Send + Sync {
    /// Stable name saved with each job.
    fn name(&self) -> &str;

    /// Whether an unfinished job of this kind continues after an Atlas restart.
    /// `false` means it is marked interrupted when Atlas starts again.
    fn resumable(&self) -> bool;

    /// Runs one step. It is dropped at any await point when the job is
    /// cancelled, so it must be cancel-safe.
    async fn run_step(&self, device: Option<&DeviceKey>, step: &str) -> Result<(), DriverError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum JobStatus {
    /// Waiting to run. Also what a resumable job becomes after a restart.
    Queued,
    Running,
    /// The last attempt at a step failed. Tried again after `retry_in_ms`.
    Retrying {
        attempt: u32,
        error: String,
        retry_in_ms: u64,
    },
    /// Held by the user. Takes effect at the next step boundary.
    Paused,
    /// Every step finished.
    Done,
    Cancelled,
    /// Atlas stopped while this ran, and it cannot continue. Nothing was
    /// resumed; start the work again.
    Interrupted {
        reason: String,
    },
}

impl JobStatus {
    fn is_finished(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Cancelled | Self::Interrupted { .. }
        )
    }
}

/// A job as saved and as reported. Steps are data: the names the kind runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: JobId,
    pub kind: String,
    pub device: Option<DeviceKey>,
    pub steps: Vec<String>,
    /// Steps finished, in order. A resumed job starts at this index.
    pub completed: usize,
    pub status: JobStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobTiming {
    /// Limit on one step. A step past it is dropped and retried.
    pub step_timeout: Duration,
    /// Wait after a failed step, doubling up to the cap.
    pub backoff: Backoff,
}

impl Default for JobTiming {
    fn default() -> Self {
        Self {
            step_timeout: Duration::from_secs(600),
            backoff: Backoff::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum JobError {
    #[error("no job kind named `{0}` in this build")]
    UnknownKind(String),
    #[error("a job needs at least one step")]
    NoSteps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Run,
    Pause,
    Cancel,
}

struct Running {
    command: watch::Sender<Command>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct State {
    /// The last job id handed out.
    last_id: u64,
    jobs: BTreeMap<JobId, JobRecord>,
    tasks: BTreeMap<JobId, Running>,
}

struct Inner {
    kinds: BTreeMap<String, Arc<dyn JobKind>>,
    path: PathBuf,
    events: Events,
    timing: JobTiming,
    state: Mutex<State>,
    /// One save at a time, each taking its snapshot under the lock.
    save_turn: AsyncMutex<()>,
}

fn lock(mutex: &Mutex<State>) -> MutexGuard<'_, State> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn record(&self, id: JobId) -> Option<JobRecord> {
        self.state().jobs.get(&id).cloned()
    }

    /// Changes one job in memory and returns its new record.
    fn set(&self, id: JobId, change: impl FnOnce(&mut JobRecord)) -> Option<JobRecord> {
        let mut state = self.state();
        let job = state.jobs.get_mut(&id)?;
        change(job);
        Some(job.clone())
    }

    /// Saves, then reports. Subscribers never see a change that is not on disk.
    async fn announce(&self, job: JobRecord) {
        self.save().await;
        self.events.emit(Event::Job { job });
    }

    async fn save(&self) {
        let _turn = self.save_turn.lock().await;
        let bytes = {
            let state = self.state();
            serde_json::to_vec_pretty(&Saved {
                format: FORMAT,
                last_id: state.last_id,
                jobs: state.jobs.values().cloned().collect(),
            })
        };
        let result = match bytes {
            Ok(bytes) => write_durably(&self.path, &bytes).await,
            Err(error) => Err(io::Error::other(error)),
        };
        if let Err(error) = result {
            self.events.emit(Event::StoreFailed {
                message: format!("{}: {error}", self.path.display()),
            });
        }
    }

    /// Changes a job's status, saves and reports it. Returns false if the
    /// job is gone.
    async fn status(&self, id: JobId, status: JobStatus) -> bool {
        match self.set(id, |job| job.status = status) {
            Some(job) => {
                self.announce(job).await;
                true
            }
            None => false,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    format: u32,
    last_id: u64,
    jobs: Vec<JobRecord>,
}

/// Writes to a temporary file, syncs it, then renames it over the old one, so
/// a power cut leaves either the old list or the new one.
async fn write_durably(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(dir).await?;
    }
    // A unique name per save: a write still in flight when Atlas stops must
    // never share a temporary file with the next one.
    static SAVES: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    let mut file = tokio::fs::File::create(&tmp).await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(&tmp, path).await
}

/// The jobs of this Atlas: submitted, run, saved and resumed.
#[derive(Clone)]
pub struct Jobs {
    inner: Arc<Inner>,
}

impl fmt::Debug for Jobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Jobs")
            .field("path", &self.inner.path)
            .finish_non_exhaustive()
    }
}

impl Jobs {
    /// Loads the saved job list from `path` (none yet is an empty list), works
    /// out what each job can do after a restart, and starts the ones that
    /// continue. A file that cannot be read or parsed is an error, not an
    /// empty list: nothing is overwritten.
    pub async fn open(
        path: impl Into<PathBuf>,
        kinds: Vec<Arc<dyn JobKind>>,
        events: Events,
        timing: JobTiming,
    ) -> io::Result<Self> {
        let path = path.into();
        let kinds: BTreeMap<String, Arc<dyn JobKind>> = kinds
            .into_iter()
            .map(|k| (k.name().to_owned(), k))
            .collect();
        let saved = read_saved(&path).await?;
        let inner = Arc::new(Inner {
            kinds,
            path,
            events,
            timing,
            state: Mutex::new(State::default()),
            save_turn: AsyncMutex::new(()),
        });

        let mut changed = false;
        let mut to_start = Vec::new();
        if let Some(saved) = saved {
            let mut state = inner.state();
            state.last_id = saved.last_id;
            for mut job in saved.jobs {
                if recover(&mut job, &inner.kinds) {
                    changed = true;
                }
                if !job.status.is_finished() {
                    let command = if job.status == JobStatus::Paused {
                        Command::Pause
                    } else {
                        Command::Run
                    };
                    to_start.push((job.id, command));
                }
                state.jobs.insert(job.id, job);
            }
            for (id, command) in to_start {
                spawn_job(&inner, &mut state, id, command);
            }
        }
        if changed {
            inner.save().await;
        }
        Ok(Self { inner })
    }

    /// Adds a job and starts it. It is saved before this returns.
    pub async fn submit(
        &self,
        kind: &str,
        device: Option<DeviceKey>,
        steps: Vec<String>,
    ) -> Result<JobId, JobError> {
        if !self.inner.kinds.contains_key(kind) {
            return Err(JobError::UnknownKind(kind.to_owned()));
        }
        if steps.is_empty() {
            return Err(JobError::NoSteps);
        }
        let record = {
            let mut state = self.inner.state();
            state.last_id += 1;
            let record = JobRecord {
                id: JobId(state.last_id),
                kind: kind.to_owned(),
                device,
                steps,
                completed: 0,
                status: JobStatus::Queued,
            };
            state.jobs.insert(record.id, record.clone());
            spawn_job(&self.inner, &mut state, record.id, Command::Run);
            record
        };
        let id = record.id;
        self.inner.announce(record).await;
        Ok(id)
    }

    pub fn list(&self) -> Vec<JobRecord> {
        self.inner.state().jobs.values().cloned().collect()
    }

    pub fn get(&self, id: JobId) -> Option<JobRecord> {
        self.inner.record(id)
    }

    /// Holds a job at its next step boundary. False if it is unknown or done.
    pub fn pause(&self, id: JobId) -> bool {
        self.send_if_active(id, Command::Pause)
    }

    /// Lets a paused job, or one waiting to retry, run now. False if it is not paused.
    pub fn resume(&self, id: JobId) -> bool {
        let paused = self
            .inner
            .record(id)
            .is_some_and(|job| job.status == JobStatus::Paused);
        paused && self.send_if_active(id, Command::Run)
    }

    /// Stops a job. A step already running is dropped; nothing after it runs.
    pub fn cancel(&self, id: JobId) -> bool {
        self.send_if_active(id, Command::Cancel)
    }

    fn send_if_active(&self, id: JobId, command: Command) -> bool {
        let state = self.inner.state();
        let active = state
            .jobs
            .get(&id)
            .is_some_and(|job| !job.status.is_finished());
        match state.tasks.get(&id) {
            Some(running) if active => {
                running.command.send_replace(command);
                true
            }
            _ => false,
        }
    }

    /// Stops every job's task right now, as if Atlas stopped: no further save
    /// happens, so the list on disk is the last one saved.
    pub fn shutdown(&self) {
        let tasks = std::mem::take(&mut self.inner.state().tasks);
        for running in tasks.into_values() {
            running.handle.abort();
        }
    }
}

async fn read_saved(path: &Path) -> io::Result<Option<Saved>> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let saved: Saved = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if saved.format != FORMAT {
        return Err(io::Error::other(format!(
            "{} is format {}, this Atlas reads format {FORMAT}",
            path.display(),
            saved.format
        )));
    }
    Ok(Some(saved))
}

/// Decides what a saved job does after a restart. Returns whether it changed.
fn recover(job: &mut JobRecord, kinds: &BTreeMap<String, Arc<dyn JobKind>>) -> bool {
    if job.status.is_finished() {
        return false;
    }
    match kinds.get(&job.kind) {
        None => {
            job.status = JobStatus::Interrupted {
                reason: format!(
                    "no job kind named `{}` in this build, so nothing can continue it",
                    job.kind
                ),
            };
            true
        }
        Some(kind) if !kind.resumable() => {
            job.status = JobStatus::Interrupted {
                reason: "Atlas stopped while this ran. This kind of work cannot continue \
                         after a restart, so nothing was resumed. Start it again."
                    .to_owned(),
            };
            true
        }
        Some(_) => {
            if job.status != JobStatus::Paused {
                job.status = JobStatus::Queued;
            }
            true
        }
    }
}

fn spawn_job(inner: &Arc<Inner>, state: &mut State, id: JobId, command: Command) {
    let (sender, receiver) = watch::channel(command);
    let owner = Arc::clone(inner);
    let handle = tokio::spawn(async move {
        run_job(Arc::clone(&owner), id, receiver).await;
        owner.state().tasks.remove(&id);
    });
    state.tasks.insert(
        id,
        Running {
            command: sender,
            handle,
        },
    );
}

/// Waits until the job is told to cancel. If the sender is gone, never returns.
async fn cancelled(command: &mut watch::Receiver<Command>) {
    loop {
        let current = *command.borrow_and_update();
        if current == Command::Cancel {
            return;
        }
        if command.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn run_job(inner: Arc<Inner>, id: JobId, mut command: watch::Receiver<Command>) {
    let mut attempt: u32 = 0;
    loop {
        let current = *command.borrow_and_update();
        match current {
            Command::Cancel => {
                inner.status(id, JobStatus::Cancelled).await;
                return;
            }
            Command::Pause => {
                inner.status(id, JobStatus::Paused).await;
                if command.changed().await.is_err() {
                    return;
                }
                continue;
            }
            Command::Run => {}
        }

        let Some(job) = inner.record(id) else {
            return;
        };
        if job.completed >= job.steps.len() {
            inner.status(id, JobStatus::Done).await;
            return;
        }
        let Some(kind) = inner.kinds.get(&job.kind).cloned() else {
            inner
                .status(
                    id,
                    JobStatus::Interrupted {
                        reason: format!("no job kind named `{}` in this build", job.kind),
                    },
                )
                .await;
            return;
        };
        let step = job.steps[job.completed].clone();
        let device = job.device.clone();
        inner.status(id, JobStatus::Running).await;

        let outcome = tokio::select! {
            biased;
            () = cancelled(&mut command) => None,
            result = guarded(inner.timing.step_timeout, async move {
                kind.run_step(device.as_ref(), &step).await
            }) => Some(result),
        };
        match outcome {
            None => {
                inner.status(id, JobStatus::Cancelled).await;
                return;
            }
            Some(Ok(())) => {
                attempt = 0;
                if let Some(job) = inner.set(id, |job| job.completed += 1) {
                    inner.announce(job).await;
                }
            }
            Some(Err(error)) => {
                attempt = attempt.saturating_add(1);
                let delay = inner.timing.backoff.delay(attempt);
                let retry_in_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
                let status = JobStatus::Retrying {
                    attempt,
                    error: error.to_string(),
                    retry_in_ms,
                };
                if !inner.status(id, status).await {
                    return;
                }
                tokio::select! {
                    () = time::sleep(delay) => {}
                    changed = command.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}
