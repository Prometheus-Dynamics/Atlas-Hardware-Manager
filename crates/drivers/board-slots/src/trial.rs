//! The trial watch on a real session: starts a trial, follows it through the board's restarts,
//! and saves what is needed so it is followed again after Atlas restarts.
//!
//! The logic is `atlas_trial` (pure). This module is the thin glue:
//!
//! - the session's presence becomes [`atlas_trial::Seen`] (a node that answers with a key other
//!   than the one the trial started on is `Replaced`, and nothing is read from it);
//! - when the board is back, its slot list is read through [`BoardSlots::list`];
//! - the saved watches live in one JSON file in the app's data directory the caller passes in.
//!
//! Nothing here confirms a trial. [`Follower::start_trial`] starts it; the board confirms itself.
//! [`Follower::stop`] is the person's way to stop following: it changes nothing on the board.
//!
//! There is no deadline. While a trial is settling and the board stays connected, its records are
//! read again every [`RECHECK`]: a poll interval, so a confirm the board makes while Atlas stays
//! connected still reaches the device list. A board that never comes back keeps its watch.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use atlas_actions::ActionError;
use atlas_session::{Handle, Presence, SessionError};
use atlas_slots::SlotList;
use atlas_trial::{
    Begun, Next, NodeKey, RUNNING_FACT, Read, Saved, Seen, TrialState, Trials, WatchStore, hex,
    key_from_hex,
};
use orion::node::Node;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::SlotError;
use crate::calls::BoardSlots;

/// How often a settling trial's records are read again while the board stays connected. A poll
/// interval, not a deadline: the board may settle for as long as it needs.
pub const RECHECK: Duration = Duration::from_secs(5);

/// The saved watches' file, in the app's data directory.
pub const FILE: &str = "trial-watches.json";

/// The format written now.
const VERSION: u32 = 1;

/// The saved watches in one JSON file, written crash-safely (temp file, sync, rename).
#[derive(Debug)]
pub struct FileWatchStore {
    path: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    watches: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    board: String,
    slot: String,
    started_ms: u64,
}

impl FileWatchStore {
    /// The store in `data_dir`. Nothing is read or created until it is used.
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(FILE),
        }
    }

    fn set_aside(&self) -> String {
        let mut name = self
            .path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        name.push(".bad");
        let aside = self.path.with_file_name(name);
        match fs::rename(&self.path, &aside) {
            Ok(()) => format!("moved to {}", aside.display()),
            Err(error) => format!("could not be moved aside: {error}"),
        }
    }
}

impl WatchStore for FileWatchStore {
    fn load(&self) -> Result<Vec<Saved>, String> {
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("cannot read {}: {error}", self.path.display())),
        };
        let parsed: Result<Document, _> = serde_json::from_slice(&data);
        let document = match parsed {
            Ok(document) if document.version == VERSION => document,
            Ok(document) => {
                return Err(format!(
                    "the saved trial watches are format {}, this build reads {VERSION}; {}",
                    document.version,
                    self.set_aside()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "the saved trial watches could not be read ({error}); {}",
                    self.set_aside()
                ));
            }
        };
        // An entry whose board key is not 64 hex digits cannot be followed: it is skipped.
        Ok(document
            .watches
            .into_iter()
            .filter_map(|entry| {
                Some(Saved {
                    board: key_from_hex(&entry.board)?,
                    slot: entry.slot,
                    started_ms: entry.started_ms,
                })
            })
            .collect())
    }

    fn save(&self, watches: &[Saved]) -> Result<(), String> {
        let document = Document {
            version: VERSION,
            watches: watches
                .iter()
                .map(|watch| Entry {
                    board: hex(&watch.board),
                    slot: watch.slot.clone(),
                    started_ms: watch.started_ms,
                })
                .collect(),
        };
        let data = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        self.write(&data)
            .map_err(|error| format!("cannot write {}: {error}", self.path.display()))
    }
}

impl FileWatchStore {
    fn write(&self, data: &[u8]) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut temp_name = self
            .path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        temp_name.push(".tmp");
        let temp = self.path.with_file_name(temp_name);
        {
            let mut file = fs::File::create(&temp)?;
            file.write_all(data)?;
            file.sync_all()?;
        }
        fs::rename(&temp, &self.path)?;
        fs::File::open(parent)?.sync_all()
    }
}

/// What a trial is started with: the slot, its two keep flags and the force flag (as
/// [`BoardSlots::start_trial`] takes them), and the caller's clock for the watch.
#[derive(Clone, Copy, Debug)]
pub struct TrialStart<'a> {
    pub id: &'a str,
    pub keep_default: bool,
    pub keep_old: bool,
    pub force: bool,
    /// The caller's clock, in milliseconds. Kept only to order the saved watches.
    pub now_ms: u64,
}

/// How a followed trial ended, and whether its release was saved.
#[derive(Debug)]
pub struct Ended {
    /// The state to report, once.
    pub state: TrialState,
    /// `Err` when the saved watches could not be written: the watch may be reported again after
    /// an Atlas restart.
    pub saved: Result<(), String>,
}

/// The trial watches of one app, followed through its board sessions.
#[derive(Debug)]
pub struct Follower {
    trials: Mutex<Trials<FileWatchStore>>,
    stops: Mutex<BTreeMap<NodeKey, oneshot::Sender<()>>>,
    recheck: Duration,
}

impl Follower {
    /// The watches saved in `data_dir`, opened. See [`Trials::open`].
    #[must_use]
    pub fn open(data_dir: &Path) -> Self {
        Self::with_store(FileWatchStore::new(data_dir))
    }

    /// The watches in `store`, opened.
    #[must_use]
    pub fn with_store(store: FileWatchStore) -> Self {
        Self {
            trials: Mutex::new(Trials::open(store)),
            stops: Mutex::new(BTreeMap::new()),
            recheck: RECHECK,
        }
    }

    /// The same, with a different poll interval for settling trials (tests use a short one).
    #[must_use]
    pub const fn with_recheck(mut self, recheck: Duration) -> Self {
        self.recheck = recheck;
        self
    }

    /// Why the saved watches could not be read at open, if they could not.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        lock(&self.trials).problem().map(str::to_owned)
    }

    /// The state of every watch held now, oldest first.
    #[must_use]
    pub fn states(&self) -> Vec<TrialState> {
        lock(&self.trials)
            .watches()
            .iter()
            .map(|watch| watch.state().clone())
            .collect()
    }

    /// Starts following a trial of `slot` on `board`, which the caller started. Returns a note for
    /// the person when the saved watches could not be written, or when an old watch was dropped to
    /// keep the bound.
    pub fn begin(&self, board: NodeKey, slot: &str, now_ms: u64) -> Option<String> {
        let begun = lock(&self.trials).begin(board, slot, now_ms);
        note(&begun)
    }

    /// Starts a trial on the board and begins following it. The trial is never retried here (see
    /// [`BoardSlots::start_trial`]). A call that did not come back may still have started it, so the
    /// board's list is read and a pending trial of `id` is followed; the error is still returned.
    ///
    /// # Errors
    ///
    /// [`SlotError`] from the call. [`SlotError::Board`] with `NotConnected` when the board is not
    /// connected: nothing was sent.
    pub async fn start_trial(
        &self,
        slots: &BoardSlots,
        handle: &Handle,
        request: TrialStart<'_>,
    ) -> Result<Option<String>, SlotError> {
        let TrialStart {
            id,
            keep_default,
            keep_old,
            force,
            now_ms,
        } = request;
        let board = handle.node_key().ok_or_else(|| {
            SlotError::Board(ActionError::NotConnected(
                "the board is not connected".into(),
            ))
        })?;
        match slots.start_trial(id, keep_default, keep_old, force).await {
            Ok(()) => Ok(self.begin(board, id, now_ms)),
            Err(error) => {
                if matches!(
                    error,
                    SlotError::Board(ActionError::Unknown { .. } | ActionError::NotFinished { .. })
                ) && let Ok(list) = slots.list().await
                    && list.trial.as_ref().is_some_and(|trial| trial.slot == id)
                {
                    self.begin(board, id, now_ms);
                }
                Err(error)
            }
        }
    }

    /// Follows the watch of `board` until it ends (held, did not hold, disagreed, replaced, or
    /// stopped), and returns its state, once. Returns `None` at once if no watch is held for
    /// `board`, if it is already followed, or if the monitor for the board is turned off.
    ///
    /// The board's presence drives it: a return reads the records. A settling trial is also read
    /// again every `recheck`. Nothing else is timed.
    pub async fn follow(
        &self,
        slots: &BoardSlots,
        handle: &Handle,
        board: NodeKey,
    ) -> Option<Ended> {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        {
            let mut stops = lock(&self.stops);
            if stops.contains_key(&board) || lock(&self.trials).watch(board).is_none() {
                return None;
            }
            stops.insert(board, stop_tx);
        }
        let ended = self.run(slots, handle, board, &mut stop_rx).await;
        lock(&self.stops).remove(&board);
        ended
    }

    /// The person stops following `board`. Nothing on the board changes. Returns the watch's
    /// final state (`Stopped`), if a watch was held.
    pub fn stop(&self, board: NodeKey) -> Option<Ended> {
        let stop = lock(&self.stops).remove(&board);
        {
            let mut trials = lock(&self.trials);
            if let Some(watch) = trials.watch_mut(board) {
                watch.stop();
            }
        }
        let ended = self.release(board);
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        ended
    }

    fn release(&self, board: NodeKey) -> Option<Ended> {
        let (state, saved) = lock(&self.trials).release(board);
        state.map(|state| Ended { state, saved })
    }

    async fn run(
        &self,
        slots: &BoardSlots,
        handle: &Handle,
        board: NodeKey,
        stop: &mut oneshot::Receiver<()>,
    ) -> Option<Ended> {
        let mut presence = handle.watch();
        loop {
            let snapshot = seen_of(&presence.borrow_and_update());
            // The running slot is read from the board's live identity, not from the facts it
            // published when this session connected: a boot that did not reconnect is seen too.
            let seen = match snapshot {
                Seen::Found { key, .. } => match live_running(handle).await {
                    Ok(running) => Seen::Found { key, running },
                    Err(_) => Seen::Away,
                },
                other => other,
            };
            let next = lock(&self.trials).watch_mut(board)?.seen(seen);
            if next == Next::ReadRecords {
                let read = read_of(slots.list().await);
                lock(&self.trials).watch_mut(board)?.read(read);
            }
            if let Some(ended) = self.release(board) {
                return Some(ended);
            }
            let settling = matches!(
                lock(&self.trials).watch(board)?.state(),
                TrialState::Settling(_)
            );
            let recheck = tokio::time::sleep(self.recheck);
            tokio::pin!(recheck);
            tokio::select! {
                _ = &mut *stop => return None,
                changed = presence.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                }
                () = &mut recheck, if settling => {}
            }
        }
    }
}

fn note(begun: &Begun) -> Option<String> {
    let mut notes = Vec::new();
    if let Err(reason) = &begun.saved {
        notes.push(format!(
            "the trial is followed now, but it is not saved, so it will not be followed after Atlas restarts: {reason}"
        ));
    }
    if let Some(evicted) = &begun.evicted {
        notes.push(format!(
            "the watch of the trial of {} was dropped to keep the saved watches within their bound",
            evicted.slot()
        ));
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// The session's presence, as the watch sees it.
fn seen_of(presence: &Presence) -> Seen {
    match presence {
        Presence::Searching | Presence::Lost(_) => Seen::Away,
        Presence::Found(node) => Seen::Found {
            key: node.public_key,
            running: running_of(node),
        },
        Presence::Replaced { expected, found } => Seen::Replaced {
            expected: *expected,
            found: *found,
        },
    }
}

/// The board's running slot, from its hello fact `version.slot`.
fn running_of(node: &Node) -> Option<String> {
    node.facts
        .iter()
        .find(|fact| fact.name == RUNNING_FACT)
        .map(|fact| String::from_utf8_lossy(&fact.value).trim().to_owned())
}

/// The running slot as the board says it now, asked on the live session.
async fn live_running(handle: &Handle) -> Result<Option<String>, SessionError> {
    handle
        .with_client(|orion| -> Result<Option<String>, orion::Error> {
            Ok(running_of(&orion.node()?))
        })
        .await
}

/// The board's answer to a read of its list, as the watch takes it. A board that did not answer
/// is "gone" (the watch waits for it); a refusal or an unreadable list is kept in words.
fn read_of(answer: Result<SlotList, SlotError>) -> Read {
    match answer {
        Ok(list) => Read::List(list),
        Err(
            error @ SlotError::Board(
                ActionError::NotConnected(_)
                | ActionError::Unknown { .. }
                | ActionError::NotFinished { .. },
            ),
        ) => Read::Gone(error.to_string()),
        Err(error) => Read::Refused(error.to_string()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
