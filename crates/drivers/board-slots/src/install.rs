//! The install of a slot, as a job: make the bundle, check the board, accept, push, install.
//!
//! Stages, in order, and what each does on a board:
//!
//! 1. **Preparing**: the bundle is made on the computer ([`Source::Image`] through
//!    `atlas_bundle::bundle_from_image`, streamed to a file in the scratch directory), or taken as
//!    given ([`Source::Bundle`]). Nothing is sent.
//! 2. **Board check**: the bundle's `board` must be this board's id. A mismatch ends the install
//!    here, before any byte or call reaches the board.
//! 3. **accept `<size>`**: the board checks there is room. Repeatable: it changes nothing, so a
//!    call that came back as unknown is sent again.
//! 4. **Sending**: the bundle is pushed (`atlas_files`) to the name `slot-<nonce>.tar`, which the
//!    board's sink takes into `.incoming`. A push resumes by itself: the board says how many bytes
//!    it holds, so acknowledged bytes are not sent again. The push is started once and kept; a
//!    retry of this step waits for that same push.
//! 5. **install_file `<name> <pin> false`**: a long action. Started once and its run number kept
//!    in the step state; a retry follows that run and never starts it again. A start that was
//!    sent and not confirmed ends the job with the words "not sent again": the board may have
//!    started it, and the person checks the list before trying again.
//! 6. **Installing**: the run's progress; the result is the new slot's id.
//!
//! **Why each step is safe to repeat.** The job step is retried when it fails without a settled
//! answer (a session lost, the node not connected). The step state lives in memory per install,
//! keyed by a nonce, so each retry resumes at the stage it stopped at:
//! - bundle: a file that exists is used; an image is written under a temporary name and renamed.
//! - board check: pure; repeating it is free.
//! - accept: repeatable (see 3).
//! - push: started once; a retry waits for that push, never submits another.
//! - install_file: started once; a retry follows the run; the start is never repeated after the
//!   node confirmed it (or was in doubt about it).
//!
//! **Not resumable across an Atlas restart.** The state is memory-only, so a job that was running
//! when Atlas stopped is marked interrupted. The board keeps a detached install it started (the
//! run goes on there), but following it after a restart needs its run number kept on disk: an
//! open item, the same as C3's action jobs.
//!
//! **Cancel.** [`SlotInstalls::cancel`] cancels the job, cancels the push, and asks the board to
//! stop its install run, then checks that the run ended cancelled. The board deletes the pushed
//! file when a job ends, so nothing is left in `.incoming`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use atlas_actions::{ActionError, ArgKind, BoardActions, CANCEL_CONFIRM, CancelState};
use atlas_driver::DriverError;
use atlas_files::{Push, PushKind, Stage as PushStage, submit as submit_push};
use atlas_jobs::{DeviceClaim, DeviceHolds, Events, JobId, JobKind, JobTiming, Jobs};
use atlas_session::{Handle, Hold, Presence, Sessions};
use atlas_slots::Stage;
use atlas_types::DeviceKey;
use orion::Address;
use serde::{Deserialize, Serialize};
use tokio::time;

use crate::SlotError;
use crate::calls::{ACCEPT, INSTALL_FILE, short};

/// The name the install job kind is saved under.
pub const KIND: &str = "slot-install";
/// What a step says when the board at the install's address is another board now.
pub const REPLACED: &str = "the board at this address was replaced by another one";
/// What an install ends with when its session lapsed while the install's hold existed: the board
/// may have dropped what the install was doing, so its state is not known from here.
pub const LAPSED: &str = "the board may have dropped this install's hold: no request reached its bridge within the lease, so what the board did with the install is not known; check the slot list before installing again";
/// The sink a pushed bundle lands in (the board's `.incoming`), as the file name's prefix.
// LOCK: Lemnos L1 (system.slots `.incoming` sink, I5e): the bridge's file sink for the device takes
// this name; today's lock names the same directory, so the prefix is data to be checked at the lock move.
pub const INCOMING: &str = "slots/.incoming/";
/// How often a push's progress is looked at while the install waits for it.
const PUSH_POLL: Duration = Duration::from_millis(20);
const PUSH_FILE_TAG: &str = "slot";

/// Where the bundle comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A bundle already made, taken as it is. Not deleted when the install ends.
    Bundle(PathBuf),
    /// A first-flash image (`.img.xz`) and the slot in it: the bundle is made here, streamed, into
    /// the scratch directory, and deleted when the install ends.
    Image {
        image: PathBuf,
        slot: String,
        product: String,
        version: String,
        variant: Option<String>,
        kernel: Option<String>,
        display_name: Option<String>,
        /// The 32-byte key that signs the bundle's `slot.sig`.
        signing_key: [u8; 32],
    },
}

/// What one install is asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallRequest {
    pub source: Source,
    /// Star the new version (the board's pin) at install. Atlas's one notion of starred.
    pub starred: bool,
}

/// The step as saved with the job. Only what a retry needs to find its run again.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Spec {
    nonce: String,
    address: String,
    board: String,
    dest: String,
    starred: bool,
    /// The node key the install was submitted to, when the node was connected then. Every step
    /// checks the node still answers with it; another key ends the install (see `same_board`).
    node_key: Option<[u8; 32]>,
    source: SavedSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum SavedSource {
    Bundle {
        path: PathBuf,
    },
    Image {
        image: PathBuf,
        slot: String,
        product: String,
        version: String,
        variant: Option<String>,
        kernel: Option<String>,
        display_name: Option<String>,
        signing_key: String,
    },
}

/// One install's state, kept in memory per nonce.
#[derive(Default)]
struct Run {
    bundle: Option<PathBuf>,
    /// Whether the bundle file is ours to delete when the install ends.
    owned: bool,
    size: u64,
    checked: bool,
    accepted: bool,
    push: Option<Push>,
    /// The push's counters, kept when the install ends (the push handle is dropped then).
    push_counters: Option<atlas_files::Counters>,
    pushed: bool,
    call: Option<u64>,
    stage: Option<Stage>,
    cancel: Option<CancelState>,
    /// The node key this install is on, recorded at its first contact when submit had none.
    node_key: Option<[u8; 32]>,
    /// Set by a cancel before the job is stopped: the step's own ending must not overwrite
    /// `Cancelled` with the board's words about the same cancel.
    cancelling: bool,
    /// The install's hold on the board: taken at its first step, kept across its steps and across
    /// a lost session, and dropped when the install ends (see `finish`).
    hold: Option<Hold>,
}

/// Why a step stopped: wait and try again, or the job ends with these words.
enum Step {
    Wait(String),
    Settled(String),
}

fn wait_of(error: &ActionError) -> Option<String> {
    match error {
        ActionError::NotConnected(reason)
        | ActionError::NotAnswering(reason)
        | ActionError::Lost { reason, .. }
        | ActionError::Unknown { reason, .. } => Some(reason.clone()),
        _ => None,
    }
}

fn classify(error: &ActionError) -> Step {
    wait_of(error).map_or_else(|| Step::Settled(error.to_string()), Step::Wait)
}

/// The install job kind. Holds the state of every install in this program.
pub(crate) struct InstallKind {
    sessions: Arc<Sessions>,
    board: String,
    scratch: PathBuf,
    push_kind: Arc<PushKind>,
    jobs: OnceLock<Jobs>,
    runs: Arc<Mutex<BTreeMap<String, Run>>>,
}

impl std::fmt::Debug for InstallKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallKind")
            .field("board", &self.board)
            .finish_non_exhaustive()
    }
}

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next_nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

impl InstallKind {
    fn runs(&self) -> MutexGuard<'_, BTreeMap<String, Run>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn with<T>(&self, nonce: &str, change: impl FnOnce(&mut Run) -> T) -> T {
        change(self.runs().entry(nonce.to_owned()).or_default())
    }

    /// Records how a step ended, unless a cancel is what ended it (the cancel records that).
    fn settle(&self, nonce: &str, stage: Stage) {
        self.with(nonce, |run| {
            if !run.cancelling {
                run.stage = Some(stage);
            }
        });
    }

    fn set_stage(&self, nonce: &str, stage: Stage) {
        self.with(nonce, |run| run.stage = Some(stage));
    }

    /// Ends an install: its state and, when Atlas made it, its bundle file are removed.
    /// The bundle file and the push handle are dropped; the stage and the cancel state stay, so
    /// the app can still say how the install ended.
    fn finish(&self, nonce: &str) {
        let mut runs = self.runs();
        let Some(run) = runs.get_mut(nonce) else {
            return;
        };
        if run.owned
            && let Some(path) = run.bundle.take()
        {
            let _ = std::fs::remove_file(path);
        }
        run.push_counters = run.push.as_ref().map(Push::counters);
        run.push = None;
        run.call = None;
        run.hold = None;
    }

    /// Step 1: the bundle, made once.
    async fn ensure_bundle(&self, spec: &Spec) -> Result<(PathBuf, u64), Step> {
        if let Some(found) = self.with_run(&spec.nonce, |run| {
            run.bundle.clone().map(|path| (path, run.size))
        }) {
            return Ok(found);
        }
        self.set_stage(&spec.nonce, Stage::Preparing);
        let (path, owned) = match &spec.source {
            SavedSource::Bundle { path } => (path.clone(), false),
            SavedSource::Image { .. } => {
                let target = self.scratch.join(format!("{}.tar", spec.nonce));
                if !target.exists() {
                    // LOCK: Lemnos L2: a signed bundle needs slot.key (the bundle writer's, D13); the
                    // bundle written here is only as signed as `atlas_bundle::bundle_from_image` makes it.
                    let image = spec.source.clone();
                    let partial = self.scratch.join(format!("{}.tar.partial", spec.nonce));
                    let scratch = self.scratch.clone();
                    let made = tokio::task::spawn_blocking(move || {
                        make_bundle(&image, &scratch, &partial)
                    })
                    .await
                    .map_err(|error| Step::Settled(format!("the bundle maker stopped: {error}")))?;
                    made.map_err(Step::Settled)?;
                    std::fs::rename(
                        self.scratch.join(format!("{}.tar.partial", spec.nonce)),
                        &target,
                    )
                    .map_err(|error| {
                        Step::Settled(format!("the bundle could not be kept: {error}"))
                    })?;
                }
                (target, true)
            }
        };
        let size = std::fs::metadata(&path)
            .map_err(|error| Step::Settled(format!("the bundle cannot be read: {error}")))?
            .len();
        self.with(&spec.nonce, |run| {
            run.bundle = Some(path.clone());
            run.owned = owned;
            run.size = size;
        });
        Ok((path, size))
    }

    fn with_run<T>(&self, nonce: &str, read: impl FnOnce(&Run) -> Option<T>) -> Option<T> {
        self.runs().get(nonce).and_then(read)
    }

    /// Step 2: the board check, on the computer. Nothing is sent when it fails.
    fn check_board(&self, spec: &Spec, bundle: &Path) -> Result<(), Step> {
        if self
            .with_run(&spec.nonce, |run| run.checked.then_some(()))
            .is_some()
        {
            return Ok(());
        }
        let found = bundle_board(bundle).map_err(Step::Settled)?;
        if found != spec.board {
            return Err(Step::Settled(
                SlotError::BoardMismatch {
                    bundle: found,
                    board: spec.board.clone(),
                }
                .to_string(),
            ));
        }
        self.with(&spec.nonce, |run| run.checked = true);
        Ok(())
    }

    /// Step 3: accept the size. Repeatable.
    async fn accept(&self, spec: &Spec, board: &BoardActions, size: u64) -> Result<(), Step> {
        if self
            .with_run(&spec.nonce, |run| run.accepted.then_some(()))
            .is_some()
        {
            return Ok(());
        }
        self.set_stage(
            &spec.nonce,
            Stage::WaitingForBoard {
                reason: "accepting the bundle".into(),
            },
        );
        board
            .run(
                &short(ACCEPT, &[("size", ArgKind::Int)]),
                &[size.to_string()],
            )
            .await
            .map_err(|error| classify(&error))?;
        self.with(&spec.nonce, |run| run.accepted = true);
        Ok(())
    }

    /// Step 4: the push, started once and waited for.
    async fn push(
        &self,
        spec: &Spec,
        handle: &Handle,
        bundle: &Path,
        size: u64,
    ) -> Result<(), Step> {
        if self
            .with_run(&spec.nonce, |run| run.pushed.then_some(()))
            .is_some()
        {
            return Ok(());
        }
        let started = self
            .with_run(&spec.nonce, |run| run.push.as_ref().map(|_| ()))
            .is_some();
        if !started {
            let jobs = self
                .jobs
                .get()
                .ok_or_else(|| Step::Settled("the job list is not open".into()))?;
            let file = File::open(bundle)
                .map_err(|error| Step::Settled(format!("the bundle cannot be opened: {error}")))?;
            let push = submit_push(
                &self.push_kind,
                jobs,
                handle,
                &format!("{INCOMING}{}", spec.dest),
                Box::new(file),
            )
            .await
            .map_err(|error| Step::Settled(error.to_string()))?;
            self.with(&spec.nonce, |run| run.push = Some(push));
        }
        loop {
            let progress = self
                .with_run(&spec.nonce, |run| run.push.as_ref().map(Push::progress))
                .ok_or_else(|| Step::Settled("the push is no longer held".into()))?;
            match progress.stage {
                PushStage::Committed => {
                    self.with(&spec.nonce, |run| run.pushed = true);
                    return Ok(());
                }
                PushStage::Failed { reason } => return Err(Step::Settled(reason)),
                PushStage::Cancelled => return Err(Step::Settled("the push was cancelled".into())),
                PushStage::WaitingForBoard { reason } => {
                    self.set_stage(&spec.nonce, Stage::WaitingForBoard { reason });
                }
                // A resend after a lapse moves bytes too: the install shows it as sending.
                PushStage::Queued | PushStage::Sending | PushStage::Resending { .. } => {
                    self.set_stage(
                        &spec.nonce,
                        Stage::Sending {
                            sent: progress.sent,
                            total: progress.total.max(size),
                        },
                    );
                }
            }
            time::sleep(PUSH_POLL).await;
        }
    }

    /// Step 5 and 6: install_file, started once, then followed to its result.
    async fn install(&self, spec: &Spec, board: &BoardActions) -> Result<String, Step> {
        let call = match self.with_run(&spec.nonce, |run| run.call) {
            Some(call) => call,
            None => {
                let words = [spec.dest.clone(), flag(spec.starred), "false".to_owned()];
                let action = short(
                    INSTALL_FILE,
                    &[
                        ("name", ArgKind::Text),
                        ("pin", ArgKind::Bool),
                        ("keep_old", ArgKind::Bool),
                    ],
                );
                match board.start(&action, &words).await {
                    Ok(call) => {
                        self.with(&spec.nonce, |run| run.call = Some(call));
                        call
                    }
                    Err(ActionError::StartInDoubt { name, reason }) => {
                        return Err(Step::Settled(format!(
                            "the board did not confirm that {name} started, so it was not sent again; check the slot list before installing again: {reason}"
                        )));
                    }
                    Err(error) => return Err(classify(&error)),
                }
            }
        };
        let runs = Arc::clone(&self.runs);
        let nonce = spec.nonce.clone();
        // Followed on the install's own hold: a lapse while the board runs the install ends it with
        // the board's words about the lapse (`ActionError::HoldLapsed`), not as a wait.
        let hold = self.with_run(&spec.nonce, |run| run.hold.clone());
        let on_progress = move |progress: &atlas_actions::Progress| {
            runs.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(nonce.clone())
                .or_default()
                .stage = Some(Stage::Installing {
                percent: progress.percent,
                message: progress.message.clone(),
            });
        };
        let followed = match &hold {
            Some(hold) => board.follow_while_held(call, hold, on_progress).await,
            None => board.follow(call, on_progress).await,
        };
        let done = followed.map_err(|error| classify(&error))?;
        String::from_utf8(done.output)
            .map_err(|_| Step::Settled("the board's result is not a slot id".into()))
    }
}

#[async_trait]
impl JobKind for InstallKind {
    fn name(&self) -> &str {
        KIND
    }

    /// Not resumed by an Atlas restart: the state is in memory. See the module docs.
    fn resumable(&self) -> bool {
        false
    }

    async fn run_step(&self, _claim: &DeviceClaim, step: &str) -> Result<(), DriverError> {
        let spec: Spec = serde_json::from_str(step).map_err(|error| {
            DriverError::Unsupported(format!("a saved install step is not one: {error}"))
        })?;
        let address = Address::from_str(&spec.address).map_err(|error| {
            DriverError::Unsupported(format!("a saved install has no node address: {error}"))
        })?;
        let Some(handle) = self.sessions.handle(&address) else {
            return Err(DriverError::Unreachable(format!(
                "{address} is turned off, so the install is not being sent"
            )));
        };
        let board = BoardActions::new(handle.clone());
        match self.drive(&spec, &handle, &board).await {
            Ok(slot) => {
                self.settle(&spec.nonce, Stage::Done { slot });
                self.finish(&spec.nonce);
                Ok(())
            }
            Err(Step::Wait(reason)) => {
                self.set_stage(
                    &spec.nonce,
                    Stage::WaitingForBoard {
                        reason: reason.clone(),
                    },
                );
                Err(DriverError::Unreachable(reason))
            }
            Err(Step::Settled(reason)) => {
                self.settle(
                    &spec.nonce,
                    Stage::Failed {
                        reason: reason.clone(),
                    },
                );
                self.finish(&spec.nonce);
                Err(DriverError::Incompatible(reason))
            }
        }
    }
}

impl InstallKind {
    async fn drive(
        &self,
        spec: &Spec,
        handle: &Handle,
        board: &BoardActions,
    ) -> Result<String, Step> {
        self.with(&spec.nonce, |run| {
            if run.hold.is_none() {
                run.hold = Some(handle.hold_work("a board install"));
            }
        });
        let (bundle, size) = self.ensure_bundle(spec).await?;
        self.check_board(spec, &bundle)?;
        self.same_board(spec, handle)?;
        self.accept(spec, board, size).await?;
        self.same_board(spec, handle)?;
        self.push(spec, handle, &bundle, size).await?;
        self.same_board(spec, handle)?;
        self.install(spec, board).await
    }

    /// The board at the address must still be the one the install started on. A replacement
    /// (another node key, or the monitor's `Replaced` presence) ends the install: nothing is sent
    /// to another board, and nothing resumes on it. The first contact records the key when submit
    /// had none. Presence is matched without a wildcard, so every state is named.
    fn same_board(&self, spec: &Spec, handle: &Handle) -> Result<(), Step> {
        // Between steps, a lapse means the board may have dropped what the install did so far.
        let lapsed = self
            .with_run(&spec.nonce, |run| run.hold.as_ref().map(Hold::lapsed))
            .unwrap_or(false);
        if lapsed {
            return Err(Step::Settled(LAPSED.to_owned()));
        }
        match handle.presence() {
            Presence::Replaced { .. } => return Err(Step::Settled(REPLACED.to_owned())),
            Presence::Searching => return Err(Step::Wait("the node is not connected yet".into())),
            Presence::Lost(reason) => return Err(Step::Wait(reason)),
            Presence::Found(_) => {}
        }
        let Some(now) = handle.node_key() else {
            return Err(Step::Wait(
                "the node has not said which key it holds yet".into(),
            ));
        };
        let expected = spec
            .node_key
            .or_else(|| self.with_run(&spec.nonce, |run| run.node_key));
        match expected {
            Some(expected) if expected != now => Err(Step::Settled(REPLACED.to_owned())),
            Some(_) => Ok(()),
            None => {
                self.with(&spec.nonce, |run| run.node_key = Some(now));
                Ok(())
            }
        }
    }
}

fn flag(value: bool) -> String {
    if value { "true".into() } else { "false".into() }
}

/// The bundle's `board`, read from its `slot.json` member. Refuses anything that is not a bundle.
fn bundle_board(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("the bundle cannot be opened: {error}"))?;
    let mut archive = tar::Archive::new(file);
    let entries = archive
        .entries()
        .map_err(|error| format!("the bundle is not a tar: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("the bundle is damaged: {error}"))?;
        let name = entry
            .path()
            .map_err(|error| format!("the bundle has a bad name: {error}"))?
            .to_string_lossy()
            .into_owned();
        if name == "slot.json" {
            let mut text = String::new();
            entry
                .take(1024 * 1024)
                .read_to_string(&mut text)
                .map_err(|error| format!("slot.json cannot be read: {error}"))?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|error| format!("slot.json is not JSON: {error}"))?;
            return value
                .get("board")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "slot.json has no board".to_owned());
        }
    }
    Err("the bundle has no slot.json".to_owned())
}

/// Makes the bundle from the image, streamed, into `partial`.
fn make_bundle(source: &SavedSource, scratch: &Path, partial: &Path) -> Result<(), String> {
    let SavedSource::Image {
        image,
        slot,
        product,
        version,
        variant,
        kernel,
        display_name,
        signing_key,
    } = source
    else {
        return Err("not an image".into());
    };
    let key = decode_key(signing_key).ok_or("the signing key is not 32 bytes of hex")?;
    let meta = atlas_bundle::SlotMeta {
        product: product.clone(),
        version: version.clone(),
        variant: variant.clone(),
        board: String::new(),
        kernel: kernel.clone(),
        display_name: display_name.clone(),
    };
    let out =
        File::create(partial).map_err(|error| format!("the bundle cannot be written: {error}"))?;
    atlas_bundle::bundle_from_image(image, scratch, slot, &meta, &key, out)
        .map(|_| ())
        .map_err(|error| format!("the bundle cannot be made from the image: {error}"))
}

fn decode_key(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut key = [0_u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(key)
}

fn encode_key(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The install jobs of one program: submitted, staged, and cancelled.
#[derive(Debug)]
pub struct SlotInstalls {
    jobs: Jobs,
    kind: Arc<InstallKind>,
}

impl SlotInstalls {
    /// Opens the job list at `path` with the install kind and the push kind registered. `board`
    /// is this board's id; `scratch` is a directory for the bundles made from images.
    ///
    /// # Errors
    ///
    /// As [`Jobs::open`].
    pub async fn open(
        path: impl Into<PathBuf>,
        sessions: Arc<Sessions>,
        events: Events,
        timing: JobTiming,
        holds: DeviceHolds,
        board: impl Into<String>,
        scratch: impl Into<PathBuf>,
    ) -> io::Result<Self> {
        let push_kind = PushKind::new();
        let kind = Arc::new(InstallKind {
            sessions,
            board: board.into(),
            scratch: scratch.into(),
            push_kind: Arc::clone(&push_kind),
            jobs: OnceLock::new(),
            runs: Arc::new(Mutex::new(BTreeMap::new())),
        });
        let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone(), push_kind];
        let jobs = Jobs::open(path, kinds, events, timing, holds).await?;
        let _ = kind.jobs.set(jobs.clone());
        Ok(Self { jobs, kind })
    }

    /// The job list, for listing and events.
    #[must_use]
    pub const fn jobs(&self) -> &Jobs {
        &self.jobs
    }

    /// Queues an install of a bundle to the node at `address`, as a job that writes to `device`.
    /// Nothing is sent by this call; the job does it.
    ///
    /// # Errors
    ///
    /// [`SlotError::NoSlots`] when the board does not offer slots (`offered` is false), or
    /// [`SlotError::Job`] when the job list refuses it.
    pub async fn submit(
        &self,
        device: DeviceKey,
        address: &Address,
        board: &str,
        offered: bool,
        request: InstallRequest,
    ) -> Result<JobId, SlotError> {
        if !offered {
            return Err(SlotError::NoSlots);
        }
        let nonce = next_nonce();
        let source = match request.source {
            Source::Bundle(path) => SavedSource::Bundle { path },
            Source::Image {
                image,
                slot,
                product,
                version,
                variant,
                kernel,
                display_name,
                signing_key,
            } => SavedSource::Image {
                image,
                slot,
                product,
                version,
                variant,
                kernel,
                display_name,
                signing_key: encode_key(&signing_key),
            },
        };
        let node_key = self
            .kind
            .sessions
            .handle(address)
            .and_then(|handle| handle.node_key());
        let spec = Spec {
            dest: format!("{PUSH_FILE_TAG}-{nonce}.tar"),
            nonce,
            address: address.to_string(),
            board: board.to_owned(),
            starred: request.starred,
            node_key,
            source,
        };
        let text =
            serde_json::to_string(&spec).map_err(|error| SlotError::Bundle(error.to_string()))?;
        self.kind.set_stage(&spec.nonce, Stage::Preparing);
        match self
            .jobs
            .submit(KIND, DeviceClaim::writes([device]), vec![text])
            .await
        {
            Ok(id) => Ok(id),
            Err(error) => {
                self.kind.finish(&spec.nonce);
                Err(SlotError::Job(error))
            }
        }
    }

    /// Where the install is, as the app shows it. `None` for a job that is not an install.
    #[must_use]
    pub fn stage(&self, job: JobId) -> Option<Stage> {
        let nonce = self.nonce_of(job)?;
        self.kind.with_run(&nonce, |run| run.stage.clone())
    }

    /// What the install's push has done over its attempts: bytes sent, attempts, and the window.
    /// `None` until the push has started.
    #[must_use]
    pub fn push_counters(&self, job: JobId) -> Option<atlas_files::Counters> {
        let nonce = self.nonce_of(job)?;
        self.kind
            .runs()
            .get(&nonce)
            .and_then(|run| run.push.as_ref().map(Push::counters).or(run.push_counters))
    }

    /// Whether a cancel reached the board's install run, once one has been asked for.
    #[must_use]
    pub fn cancel_state(&self, job: JobId) -> Option<CancelState> {
        let nonce = self.nonce_of(job)?;
        self.kind.with_run(&nonce, |run| run.cancel.clone())
    }

    /// Cancels the install: the job, the push, and the board's install run (which the board
    /// confirms cancelled). Returns false when the job is not active.
    pub async fn cancel(&self, job: JobId) -> bool {
        let Some(record) = self.jobs.get(job) else {
            return false;
        };
        let Some(spec) = record
            .steps
            .first()
            .and_then(|step| serde_json::from_str::<Spec>(step).ok())
        else {
            return false;
        };
        self.kind.with(&spec.nonce, |run| run.cancelling = true);
        if !self.jobs.cancel(job) {
            return false;
        }
        let call = self.kind.with_run(&spec.nonce, |run| run.call);
        if let Some(push) = self
            .kind
            .runs()
            .get(&spec.nonce)
            .and_then(|run| run.push.as_ref())
        {
            push.cancel();
        }
        let state = match call {
            Some(call) => stop_on_board(&self.kind, &spec, call).await,
            None => CancelState::NoRun,
        };
        self.kind.with(&spec.nonce, |run| {
            run.cancel = Some(state);
            run.stage = Some(Stage::Cancelled);
        });
        self.kind.finish(&spec.nonce);
        true
    }

    fn nonce_of(&self, job: JobId) -> Option<String> {
        let record = self.jobs.get(job)?;
        let spec: Spec = serde_json::from_str(record.steps.first()?).ok()?;
        Some(spec.nonce)
    }
}

async fn stop_on_board(kind: &InstallKind, spec: &Spec, call: u64) -> CancelState {
    let Ok(address) = Address::from_str(&spec.address) else {
        return CancelState::NotConfirmed(format!("{} is not an address", spec.address));
    };
    let Some(handle) = kind.sessions.handle(&address) else {
        return CancelState::NotConfirmed(format!(
            "{address} is turned off; install run {call} may still be going"
        ));
    };
    let board = BoardActions::new(handle);
    if let Err(error) = board.cancel(call).await {
        return CancelState::NotConfirmed(format!(
            "the cancel did not reach the board ({error}); install run {call} may still be going"
        ));
    }
    match time::timeout(CANCEL_CONFIRM, board.follow(call, |_| {})).await {
        Ok(Err(ActionError::Cancelled(_))) => CancelState::Confirmed,
        Ok(Ok(_)) => {
            CancelState::NotConfirmed("the install finished before the cancel took effect".into())
        }
        Ok(Err(other)) => CancelState::NotConfirmed(other.to_string()),
        Err(_) => {
            CancelState::NotConfirmed(format!("the board did not confirm that run {call} stopped"))
        }
    }
}
