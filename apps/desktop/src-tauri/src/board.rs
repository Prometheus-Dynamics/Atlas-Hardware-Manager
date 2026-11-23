//! The board page's work (F4): one board's readings, hardware, logs, actions and slots, read
//! through its Orion node's session and published as the topic `board:<id>`.
//!
//! A page that opens calls `board.open`, which starts a reader for that board, and calls it
//! again every few seconds while it is shown; the reader stops by itself once nobody has asked
//! for a while, so a page that went away without saying so leaves nothing running. The reader
//! never blocks a call: every call here returns once the board answered or refused, and the
//! state it changed arrives with the next publish.
//!
//! Everything goes through the libraries: `atlas-topics` (readings, hardware, logs),
//! `atlas-actions` (what the board offers to run, read from its resource list and its devices'
//! descriptions), `atlas-board-slots` (slots, installs, trials).
//! What a library does not have is not made up here: the page says it is not in this build.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use atlas_actions::{Action, ActionError, BoardActions, Reported as ActionReported, Route};
use atlas_board_slots::{
    BoardSlots, Follower, InstallRequest, SlotError, SlotInstalls, Source, TrialStart,
};
use atlas_jobs::{JobId, JobTiming};
use atlas_session::{Handle, Presence};
use atlas_slots::{Resolution, SlotList, Stage, TrialLast};
use atlas_topics::description::DescriptionState;
use atlas_topics::{Connection, Hardware, Reported, Scalar, Snapshot};
use atlas_types::{CapabilityKind, CapabilitySet, DeviceKey, LinkId, Reading as Granted};
use orion::Address;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::call::{CallError, CallResult, args};
use crate::core::{Core, now_ms};

/// How often an open board is read.
const READ_EVERY: Duration = Duration::from_secs(1);
/// A reader with nobody asking for this long stops.
const KEEP_FOR: Duration = Duration::from_secs(12);
/// Samples kept per reading: the last minute at one read a second.
const HISTORY: usize = 60;
/// Log lines sent to the page, newest first.
const LOG_LINES: usize = 120;

/// The topic one board's page reads.
#[must_use]
pub fn topic(id: &str) -> String {
    format!("board:{id}")
}

/// Asks the person for a file. The window opens the system's file dialog; a build without a
/// window (the UI bridge) answers with the file the simulated world queued, or with none.
pub trait Picker: Send + Sync {
    /// The chosen file, or `None` when the person chose nothing.
    fn pick(&self, title: &str, extensions: &[&str]) -> Option<PathBuf>;
}

/// The last thing an action or a slot call said, shown on the page until the next one.
#[derive(Clone, Debug)]
struct Said {
    what: String,
    ok: bool,
    words: String,
    at_ms: u64,
}

/// One install the page follows.
#[derive(Clone, Debug)]
struct Following {
    job: JobId,
    file: String,
}

/// One open board.
struct Open {
    asked: Instant,
    dirty: Arc<Notify>,
    said: Option<Said>,
    install: Option<Following>,
}

/// The state the board pages share. Held by the core.
pub struct Boards {
    open: Mutex<BTreeMap<String, Open>>,
    installs: tokio::sync::OnceCell<Option<Arc<SlotInstalls>>>,
    follower: Option<Arc<Follower>>,
    picker: Mutex<Option<Arc<dyn Picker>>>,
    /// One hardware reader per node address, kept for the app's life: it holds the node's
    /// subscriptions and its log ring, so a page that is opened again still has the lines.
    hardware: Mutex<BTreeMap<String, Arc<Hardware>>>,
    /// The version of each board's default slot, as last read: the device list's Default column.
    defaults: Mutex<BTreeMap<String, String>>,
}

impl std::fmt::Debug for Boards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Boards").finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Boards {
    #[must_use]
    pub fn new(data_dir: Option<&std::path::Path>) -> Self {
        Self {
            open: Mutex::new(BTreeMap::new()),
            installs: tokio::sync::OnceCell::new(),
            follower: data_dir.map(|dir| Arc::new(Follower::open(dir))),
            picker: Mutex::new(None),
            hardware: Mutex::new(BTreeMap::new()),
            defaults: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn set_picker(&self, picker: Arc<dyn Picker>) {
        *lock(&self.picker) = Some(picker);
    }

    /// What asks the person for a file, when the app has a way to.
    #[must_use]
    pub fn picker(&self) -> Option<Arc<dyn Picker>> {
        lock(&self.picker).clone()
    }

    /// The default slot's version of each board whose slots were read.
    #[must_use]
    pub fn defaults(&self) -> BTreeMap<String, String> {
        lock(&self.defaults).clone()
    }
}

/// What the page needs to reach one board.
struct Target {
    id: String,
    key: DeviceKey,
    address: String,
    link: LinkId,
    family: String,
    capabilities: CapabilitySet,
}

fn target(core: &Core, id: &str) -> Result<Target, CallError> {
    let key =
        DeviceKey::parse(id).ok_or_else(|| CallError(format!("`{id}` is not a device key")))?;
    core.with_inventory(|inventory| {
        let record = inventory
            .get(&key)
            .ok_or_else(|| CallError(format!("no device {id}")))?;
        Ok(Target {
            id: id.to_owned(),
            key: record.key.clone(),
            address: record.identity.address.clone(),
            link: record.identity.link.clone(),
            family: record
                .key
                .family()
                .map(|family| family.as_str().to_owned())
                .unwrap_or_default(),
            capabilities: record.capabilities.clone(),
        })
    })
}

/// The session the scan keeps with this board, when it is an Orion node the scan has reached.
fn handle(core: &Core, target: &Target) -> Option<Handle> {
    core.sessions()?.existing(&target.address)
}

fn connected(core: &Core, target: &Target) -> Result<Handle, CallError> {
    handle(core, target)
        .ok_or_else(|| CallError("this board is not reached through Orion now".to_owned()))
}

fn slots_of(handle: Handle, target: &Target) -> BoardSlots {
    BoardSlots::for_capabilities(handle, target.family.clone(), &target.capabilities)
}

fn action_words(error: &ActionError) -> String {
    error
        .board_reason()
        .map_or_else(|| error.to_string(), str::to_owned)
}

fn slot_words(error: &SlotError) -> String {
    match error {
        SlotError::Board(error) => action_words(error),
        other => other.to_string(),
    }
}

// ----- what is published -----------------------------------------------------------------------

/// The samples of every reading of one board, by device and item.
#[derive(Default)]
struct History {
    series: BTreeMap<(String, String), VecDeque<f64>>,
}

impl History {
    fn note(&mut self, device: &str, item: &str, value: f64) -> Vec<f64> {
        let series = self
            .series
            .entry((device.to_owned(), item.to_owned()))
            .or_default();
        series.push_back(value);
        while series.len() > HISTORY {
            series.pop_front();
        }
        series.iter().copied().collect()
    }
}

fn reported<T: Clone + Into<Value>>(value: &Reported<T>) -> Value {
    value.get().cloned().map_or(Value::Null, Into::into)
}

fn hardware_json(snapshot: &Snapshot, history: &mut History, live: bool) -> Value {
    let devices: Vec<Value> = snapshot
        .devices
        .iter()
        .map(|device| {
            let readings: Vec<Value> = device
                .readings
                .iter()
                .map(|reading| {
                    let (number, text) = match &reading.value {
                        Some(Scalar::Number(number)) => (Some(*number), None),
                        Some(Scalar::Switch(on)) => {
                            (None, Some(if *on { "on" } else { "off" }.to_owned()))
                        }
                        Some(Scalar::Text(text)) => (None, Some(text.clone())),
                        Some(Scalar::Numbers(numbers)) => (
                            None,
                            Some(
                                numbers
                                    .iter()
                                    .map(|n| format!("{n:.2}"))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            ),
                        ),
                        None => (None, None),
                    };
                    // A sample is added only on a read that was made: a stale read repeats the
                    // last value and would draw a flat line that was never measured.
                    let series = match number {
                        Some(number) if live => history.note(&device.name, &reading.name, number),
                        _ => history
                            .series
                            .get(&(device.name.clone(), reading.name.clone()))
                            .map(|series| series.iter().copied().collect())
                            .unwrap_or_default(),
                    };
                    json!({
                        "name": reading.name,
                        "number": number,
                        "text": text,
                        "unit": reported(&reading.unit),
                        "kind": reading.reading.get().map(|kind| format!("{kind:?}").to_lowercase()),
                        "low": reading.range.get().map(|range| range.min),
                        "high": reading.range.get().map(|range| range.max),
                        "warn_low": reading.warn_low.get(),
                        "warn_high": reading.warn_high.get(),
                        "settable": reading.settable.get(),
                        "history": series,
                    })
                })
                .collect();
            json!({
                "name": device.name,
                "shown": device.shown_name(),
                "bus": reported(&device.bus),
                "kind": reported(&device.kind),
                "state": device.state.get().map(|state| format!("{state:?}").to_lowercase()),
                "reason": reported(&device.state_reason),
                "health": reported(&device.health),
                "listed": device.listed,
                // How long the board has not answered this device's description, as the time stands now
                // (rebuilt on every read, so it counts up); null when it is answering or off.
                "not_answering_for": match &device.description {
                    DescriptionState::NotYet { since, .. } => {
                        Value::String(atlas_session::words::duration(since.elapsed()))
                    }
                    _ => Value::Null,
                },
                "readings": readings,
            })
        })
        .collect();
    Value::Array(devices)
}

fn slots_json(list: &SlotList) -> Value {
    let slots: Vec<Value> = list
        .slots
        .iter()
        .map(|slot| {
            json!({
                "id": slot.id,
                "product": slot.product,
                "board": slot.board,
                "version": slot.version,
                "display_name": slot.display_name,
                "good": slot.good,
                "signed": slot.signed,
                "size": slot.size,
                "pinned": slot.pinned,
                "failed": slot.failed,
                "failures": slot.failures,
                "running": slot.id == list.running,
                "default": slot.id == list.default,
                "previous": slot.id == list.previous,
                "trial": slot.id == list.trial_slot,
            })
        })
        .collect();
    let trial = list.trial.as_ref().map(|trial| {
        json!({
            "slot": trial.slot,
            "attempts_left": trial.attempts_left,
            "attempts": trial.attempts.iter().map(|a| json!({ "n": a.n, "time": a.time })).collect::<Vec<_>>(),
            "last": trial.last.as_ref().map(|last| match last {
                TrialLast::PowerLostBeforeDaemon => "power lost before the board's daemon started".to_owned(),
                TrialLast::DaemonReachedNotYetConfirmed => "started, not confirmed yet".to_owned(),
                TrialLast::Confirmed => "confirmed".to_owned(),
                TrialLast::FellBack => "fell back".to_owned(),
                TrialLast::Other(word) => word.clone(),
            }),
            "last_time": trial.last_time,
            "reason": trial.reason,
            "record_error": trial.record_error,
        })
    });
    let resolved = list.resolved.as_ref().map(|resolved| {
        json!({
            "slot": resolved.slot,
            "outcome": match &resolved.outcome {
                Resolution::Confirmed => "confirmed".to_owned(),
                Resolution::FellBack => "fell-back".to_owned(),
                Resolution::Other(word) => word.clone(),
            },
            "by": resolved.by,
            "caller": resolved.caller,
            "reason": resolved.reason,
            "time": resolved.time,
        })
    });
    json!({
        "running": list.running,
        "default": list.default,
        "previous": list.previous,
        "trial_slot": list.trial_slot,
        "state": list.state,
        "error": list.error,
        "free_bytes": list.free_bytes,
        "keep": list.keep,
        "slots": slots,
        "trial": trial,
        "resolved": resolved,
    })
}

fn stage_json(stage: &Stage, file: &str) -> Value {
    let (word, percent, detail, done): (&str, Option<f64>, String, bool) = match stage {
        Stage::Preparing => ("preparing", None, "making the slot bundle".into(), false),
        Stage::WaitingForBoard { reason } => ("waiting", None, reason.clone(), false),
        Stage::Sending { sent, total } => (
            "sending",
            #[allow(clippy::cast_precision_loss)]
            Some(if *total == 0 {
                0.0
            } else {
                *sent as f64 * 100.0 / *total as f64
            }),
            format!("{sent} of {total} bytes sent"),
            false,
        ),
        Stage::Installing { percent, message } => {
            ("installing", percent.map(f64::from), message.clone(), false)
        }
        Stage::Unknown { reason, checking } => (
            "unknown",
            None,
            format!("{reason}; checking {checking}"),
            false,
        ),
        Stage::Done { slot } => ("done", Some(100.0), slot.clone(), true),
        Stage::Failed { reason } => ("failed", None, reason.clone(), true),
        Stage::Cancelled => ("cancelled", None, String::new(), true),
    };
    json!({ "stage": word, "percent": percent, "detail": detail, "final": done, "file": file,
            "sent": match stage { Stage::Sending { sent, .. } => Some(*sent), _ => None },
            "total": match stage { Stage::Sending { total, .. } => Some(*total), _ => None } })
}

fn actions_json(actions: &[Action]) -> Value {
    Value::Array(
        actions
            .iter()
            .map(|action| {
                json!({
                    "name": action.name,
                    "label": match &action.label { ActionReported::Known(label) => Some(label.clone()), ActionReported::NotReported => None },
                    "description": match &action.description { ActionReported::Known(text) => Some(text.clone()), ActionReported::NotReported => None },
                    "direct": action.route() == Route::Direct,
                    "args": match &action.args { ActionReported::Known(args) => Some(args.len()), ActionReported::NotReported => None },
                })
            })
            .collect(),
    )
}

// ----- the reader ------------------------------------------------------------------------------

/// Starts (or keeps) the reader of one board.
fn open(core: &Core, id: &str) -> Result<(), CallError> {
    let target = target(core, id)?;
    let boards = core.boards();
    {
        let mut open = lock(&boards.open);
        if let Some(entry) = open.get_mut(id) {
            entry.asked = Instant::now();
            return Ok(());
        }
        open.insert(
            id.to_owned(),
            Open {
                asked: Instant::now(),
                dirty: Arc::new(Notify::new()),
                said: None,
                install: None,
            },
        );
    }
    tokio::spawn(read(core.clone(), target));
    Ok(())
}

/// Reads one board until nobody asks for it any more.
async fn read(core: Core, first: Target) {
    let id = first.id.clone();
    let topic = topic(&id);
    let mut history = History::default();
    let mut read_at = String::new();
    let mut catalog: Option<Vec<Action>> = None;
    let mut slots: Value = Value::Null;
    let mut tick: u64 = 0;
    loop {
        let (dirty, said, following) = {
            let mut open = lock(&core.boards().open);
            let Some(entry) = open.get(&id) else { break };
            if entry.asked.elapsed() > KEEP_FOR {
                open.remove(&id);
                break;
            }
            (
                Arc::clone(&entry.dirty),
                entry.said.clone(),
                entry.install.clone(),
            )
        };
        // The record may have moved to another link or address since the page opened.
        let Ok(target) = target(&core, &id) else {
            break;
        };
        let handle = handle(&core, &target);
        let mut view = json!({
            "id": id,
            "reach": if handle.is_some() { "orion" } else { "none" },
            "said": said.map(|said| json!({ "what": said.what, "ok": said.ok, "words": said.words, "at_ms": said.at_ms })),
            "read_ms": now_ms(),
        });
        if let Some(handle) = handle {
            let found = matches!(handle.presence(), Presence::Found(_));
            if read_at != target.address {
                read_at.clone_from(&target.address);
                history = History::default();
            }
            let reader = Arc::clone(
                lock(&core.boards().hardware)
                    .entry(target.address.clone())
                    .or_insert_with(|| {
                        Arc::new(Hardware::new(handle.clone(), target.link.clone()))
                    }),
            );
            let snapshot = reader.snapshot().await;
            let live = snapshot.connection == Connection::Live;
            view["connection"] = match &snapshot.connection {
                Connection::Live => json!({ "live": true }),
                Connection::Stale { since, reason } => json!({
                    "live": false,
                    "reason": reason,
                    "stale_ms": u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX),
                }),
            };
            view["hardware"] = hardware_json(&snapshot, &mut history, live);
            view["logs"] = Value::Array(
                snapshot
                    .logs
                    .iter()
                    .rev()
                    .take(LOG_LINES)
                    .map(|line| json!({ "time_ms": line.time / 1_000_000, "text": line.text }))
                    .collect(),
            );
            view["logs_missed"] = json!(snapshot.logs_missed);

            // What the board offers to run: the methods its devices describe, under the devices its
            // resource list gives a `call`. Read with every sample, so a device that comes or goes
            // is seen; the descriptions are the session's kept ones, so no device is asked for it.
            if found {
                match BoardActions::new(handle.clone()).catalog().await {
                    Ok(list) => catalog = Some(list.actions().to_vec()),
                    Err(error) => {
                        catalog = None;
                        view["actions_error"] = json!(action_words(&error));
                    }
                }
            }
            view["actions"] = catalog.as_deref().map_or(Value::Null, actions_json);

            // Slots: every other read, and at once after a call that changed them.
            let board_slots = slots_of(handle.clone(), &target);
            if found && (tick.is_multiple_of(2) || slots.is_null()) {
                slots = match target.capabilities.reading(CapabilityKind::Slots) {
                    Granted::NotGranted => json!({ "state": "none" }),
                    Granted::NotReported => json!({ "state": "not-reported" }),
                    Granted::Granted => match board_slots.list().await {
                        Ok(list) => {
                            if let Some(default) = list.get(&list.default) {
                                let changed = lock(&core.boards().defaults)
                                    .insert(id.clone(), default.version.clone())
                                    .as_deref()
                                    != Some(default.version.as_str());
                                if changed {
                                    core.republish_inventory();
                                }
                            }
                            json!({ "state": "ok", "list": slots_json(&list) })
                        }
                        Err(error) => json!({ "state": "error", "error": slot_words(&error) }),
                    },
                };
            }
            view["slots"] = slots.clone();
            if let Some(following) = following
                && let Some(Some(installs)) = core.boards().installs.get()
                && let Some(stage) = installs.stage(following.job)
            {
                view["install"] = stage_json(&stage, &following.file);
            }
        }
        core.publish(&topic, view);
        tick += 1;
        tokio::select! {
            () = tokio::time::sleep(READ_EVERY) => {}
            () = dirty.notified() => { slots = Value::Null; }
        }
    }
    core.retire(&topic);
}

/// Notes what a call said and wakes the board's reader, so the page shows it at once.
fn say(core: &Core, id: &str, what: &str, result: &Result<String, String>) {
    if let Some(entry) = lock(&core.boards().open).get_mut(id) {
        entry.said = Some(Said {
            what: what.to_owned(),
            ok: result.is_ok(),
            words: match result {
                Ok(words) | Err(words) => words.clone(),
            },
            at_ms: now_ms(),
        });
        entry.dirty.notify_one();
    }
}

fn finish(core: &Core, id: &str, what: &str, result: Result<String, String>) -> CallResult {
    say(core, id, what, &result);
    result.map(Value::String).map_err(CallError)
}

async fn installs(core: &Core) -> Result<Arc<SlotInstalls>, CallError> {
    let opened = core
        .boards()
        .installs
        .get_or_init(|| async {
            let sessions = core.sessions()?.clone();
            let dir = core.dirs()?.data.join("slot-installs");
            std::fs::create_dir_all(&dir).ok()?;
            SlotInstalls::open(
                dir.join("jobs.json"),
                sessions,
                core.events().clone(),
                JobTiming::default(),
                core.holds().clone(),
                String::new(),
                dir.join("scratch"),
            )
            .await
            .map_err(|error| {
                core.log()
                    .failed_call("slot-installs.open", "", &error.to_string());
            })
            .ok()
            .map(Arc::new)
        })
        .await;
    opened.clone().ok_or_else(|| {
        CallError("installs cannot be saved on this computer, so none is started".into())
    })
}

/// Follows a trial to its outcome in the background. The board decides; Atlas only reads.
fn follow_trial(core: &Core, id: &str, slots: BoardSlots, handle: Handle) {
    let (Some(follower), Some(key)) = (core.boards().follower.clone(), handle.node_key()) else {
        return;
    };
    let (core, id) = (core.clone(), id.to_owned());
    tokio::spawn(async move {
        if let Some(ended) = follower.follow(&slots, &handle, key).await {
            core.log()
                .breadcrumb(&format!("trial on {id} ended: {:?}", ended.state));
            if let Some(entry) = lock(&core.boards().open).get(&id) {
                entry.dirty.notify_one();
            }
        }
    });
}

#[derive(Deserialize)]
struct OnBoard {
    id: String,
}

#[derive(Deserialize)]
struct OnSlot {
    id: String,
    slot: String,
}

/// The `board.*` calls.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    match name {
        "board.open" => {
            let OnBoard { id } = args(name, arguments)?;
            open(core, &id)?;
            Ok(Value::Null)
        }
        "board.close" => {
            let OnBoard { id } = args(name, arguments)?;
            lock(&core.boards().open).remove(&id);
            Ok(Value::Null)
        }
        // Runs one short action the board offers, by its name in the list read from the board.
        "board.action" => {
            #[derive(Deserialize)]
            struct Run {
                id: String,
                action: String,
            }
            let Run { id, action } = args(name, arguments)?;
            let target = target(core, &id)?;
            let actions = BoardActions::new(connected(core, &target)?);
            core.log().breadcrumb(&format!("{action} on {id}"));
            let result = async {
                let catalog = actions.catalog().await.map_err(|e| action_words(&e))?;
                let entry = catalog
                    .get(&action)
                    .ok_or_else(|| format!("this board does not offer {action}"))?;
                let done = actions
                    .run(entry, &[])
                    .await
                    .map_err(|e| action_words(&e))?;
                let output = String::from_utf8_lossy(&done.output).trim().to_owned();
                Ok(if output.is_empty() {
                    done.detail
                } else {
                    output
                })
            }
            .await;
            finish(core, &id, &action, result)
        }
        "board.slots.default"
        | "board.slots.remove"
        | "board.slots.pin"
        | "board.slots.unpin"
        | "board.slots.trial" => {
            let OnSlot { id, slot } = args(name, arguments)?;
            let target = target(core, &id)?;
            let handle = connected(core, &target)?;
            let slots = slots_of(handle.clone(), &target);
            core.log().breadcrumb(&format!("{name} {slot} on {id}"));
            let result = match name {
                "board.slots.default" => slots.set_default(&slot, false).await,
                "board.slots.remove" => slots.remove(&slot, false).await,
                "board.slots.pin" => slots.set_starred(&slot, true).await,
                "board.slots.unpin" => slots.set_starred(&slot, false).await,
                _ => match core.boards().follower.clone() {
                    Some(follower) => {
                        let started = follower
                            .start_trial(
                                &slots,
                                &handle,
                                TrialStart {
                                    id: &slot,
                                    keep_default: false,
                                    keep_old: false,
                                    force: false,
                                    now_ms: now_ms(),
                                },
                            )
                            .await
                            .map(drop);
                        follow_trial(core, &id, slots.clone(), handle.clone());
                        started
                    }
                    None => slots.start_trial(&slot, false, false, false).await,
                },
            };
            let what = name.trim_start_matches("board.slots.");
            finish(
                core,
                &id,
                &format!("{what} {slot}"),
                result.map(|()| String::new()).map_err(|e| slot_words(&e)),
            )
        }
        "board.slots.keep" => {
            #[derive(Deserialize)]
            struct Keep {
                id: String,
                keep: u32,
            }
            let Keep { id, keep } = args(name, arguments)?;
            let target = target(core, &id)?;
            let slots = slots_of(connected(core, &target)?, &target);
            let result = slots.set_keep(keep).await;
            finish(
                core,
                &id,
                &format!("keep last {keep}"),
                result.map(|()| String::new()).map_err(|e| slot_words(&e)),
            )
        }
        // Asks for a slot bundle and installs it as a job. Answers null when no file was chosen.
        "board.slots.install" => {
            let OnBoard { id } = args(name, arguments)?;
            let target = target(core, &id)?;
            let handle = connected(core, &target)?;
            let picker = lock(&core.boards().picker).clone();
            let Some(picker) = picker else {
                return Err(CallError("this build has no way to choose a file".into()));
            };
            let chosen =
                tokio::task::spawn_blocking(move || picker.pick("Install a slot", &["tar"]))
                    .await
                    .map_err(|error| CallError(error.to_string()))?;
            let Some(path) = chosen else {
                return Ok(Value::Null);
            };
            let file = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let installs = installs(core).await?;
            let address: Address = target
                .address
                .parse()
                .map_err(|_| CallError(format!("{} is not an Orion address", target.address)))?;
            drop(handle);
            let offered = target.capabilities.reading(CapabilityKind::Slots) == Granted::Granted;
            core.log().breadcrumb(&format!("install {file} on {id}"));
            let job = installs
                .submit(
                    target.key.clone(),
                    &address,
                    &target.family,
                    offered,
                    InstallRequest {
                        source: Source::Bundle(path),
                        starred: false,
                    },
                )
                .await
                .map_err(|error| CallError(slot_words(&error)))?;
            if let Some(entry) = lock(&core.boards().open).get_mut(&id) {
                entry.install = Some(Following { job, file });
                entry.dirty.notify_one();
            }
            Ok(json!({ "job": job.0 }))
        }
        // Clears a finished install's line from the page.
        "board.slots.install.dismiss" => {
            let OnBoard { id } = args(name, arguments)?;
            if let Some(entry) = lock(&core.boards().open).get_mut(&id) {
                entry.install = None;
                entry.dirty.notify_one();
            }
            Ok(Value::Null)
        }
        "board.slots.install.cancel" => {
            let OnBoard { id } = args(name, arguments)?;
            let job = lock(&core.boards().open)
                .get(&id)
                .and_then(|entry| entry.install.as_ref().map(|following| following.job));
            if let (Some(job), Some(Some(installs))) = (job, core.boards().installs.get()) {
                installs.cancel(job).await;
            }
            Ok(Value::Null)
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}
