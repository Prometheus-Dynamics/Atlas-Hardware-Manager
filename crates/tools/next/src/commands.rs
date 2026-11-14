//! The commands. Each one either says what happened and succeeds, or refuses with a status and
//! the words to show. The words of the board, the install and the trial are passed on as they are.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atlas_board_slots::{
    BoardSlots, Follower, InstallRequest, SlotError, SlotInstalls, Source, TrialStart,
};
use atlas_jobs::{DeviceHolds, Events, JobTiming};
use atlas_slots::{SlotList, Stage};
use atlas_trial::{Running, TrialState};
use atlas_types::DeviceKey;
use orion::Address;
use orion::node::Node;
use serde_json::json;

use crate::args::{Command, Verb};
use crate::link::{self, BoardInfo, Link};
use crate::pins::{Pin, Pins};
use crate::report::Report;
use crate::{Env, parse_address};

/// How often an install's stage is looked at while it runs.
const POLL: Duration = Duration::from_millis(50);

/// A command that did not succeed: its status and the words to show.
#[derive(Debug)]
pub struct Refusal {
    pub code: i32,
    pub words: String,
}

impl Refusal {
    pub fn new(code: i32, words: impl Into<String>) -> Self {
        Self {
            code,
            words: words.into(),
        }
    }
}

impl From<SlotError> for Refusal {
    fn from(error: SlotError) -> Self {
        Self::new(1, error.to_string())
    }
}

/// Runs a parsed command and returns its status.
pub async fn run<W: Write>(command: Command, env: &Env, out: &mut W) -> i32 {
    let mut r = Report::new(out, command.json);
    let address = match parse_address(&command.address) {
        Ok(address) => address,
        Err(why) => {
            r.say(why);
            return r.finish(2);
        }
    };
    let Some(state) = command.state.clone().or_else(|| env.state_dir.clone()) else {
        r.say("no state directory: give --state <dir>");
        return r.finish(2);
    };
    let outcome = match &command.verb {
        Verb::Fingerprint => fingerprint(&mut r, env, &state, &address).await,
        Verb::Pin => pin(&mut r, env, &state, &address).await,
        Verb::Repin => repin(&mut r, env, &state, &address, command.reflashed).await,
        verb => board(&mut r, env, &state, &address, verb).await,
    };
    match outcome {
        Ok(()) => r.finish(0),
        Err(refusal) => {
            r.say(refusal.words);
            r.finish(refusal.code)
        }
    }
}

async fn fingerprint<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
) -> Result<(), Refusal> {
    let text = address.to_string();
    let node = probe_or_refuse(env, address, &text).await?;
    r.say(format!("{text}: key fingerprint {}", node.fingerprint));
    let pins = load_pins(state)?;
    match pins.get(&text) {
        None => r.say("not pinned: run `pin` to trust this key"),
        Some(pin) if pin.key == node.public_key => {
            r.say("pinned: this is the key the address is pinned to");
        }
        Some(pin) => r.say(format!(
            "not the pinned key: pinned as {}; a reflash is the one legitimate cause",
            pin.fingerprint
        )),
    }
    r.field("address", text);
    r.field("fingerprint", node.fingerprint);
    Ok(())
}

async fn pin<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
) -> Result<(), Refusal> {
    let text = address.to_string();
    let node = probe_or_refuse(env, address, &text).await?;
    let mut pins = load_pins(state)?;
    match pins.get(&text).cloned() {
        Some(pin) if pin.key == node.public_key => {
            r.say(format!("already pinned: {}", node.fingerprint));
        }
        Some(pin) => return Err(link::replaced(&text, &pin, Some(node.fingerprint.clone()))),
        None => {
            pins.set(Pin {
                address: text.clone(),
                key: node.public_key,
                fingerprint: node.fingerprint.clone(),
            });
            save_pins(&pins)?;
            r.say(format!(
                "pinned {} for {text}: every command now checks this key",
                node.fingerprint
            ));
        }
    }
    r.field("fingerprint", node.fingerprint);
    Ok(())
}

async fn repin<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    reflashed: bool,
) -> Result<(), Refusal> {
    let text = address.to_string();
    let node = probe_or_refuse(env, address, &text).await?;
    let mut pins = load_pins(state)?;
    let old = pins.get(&text).cloned();
    if !reflashed {
        let pinned = old
            .as_ref()
            .map_or_else(|| "none".to_owned(), |pin| pin.fingerprint.clone());
        return Err(Refusal::new(
            3,
            format!(
                "repin replaces the pin, and is for a reflashed board only. Pinned: {pinned}. Answering now: {}. Nothing was changed. If the board was reflashed, run it again with --reflashed.",
                node.fingerprint
            ),
        ));
    }
    match &old {
        Some(pin) if pin.key == node.public_key => {
            r.say(format!("unchanged: {} is still the pin", node.fingerprint));
        }
        Some(pin) => r.say(format!(
            "repinned {text}: {} -> {}",
            pin.fingerprint, node.fingerprint
        )),
        None => r.say(format!(
            "pinned {text} as {} (it had no pin)",
            node.fingerprint
        )),
    }
    pins.set(Pin {
        address: text,
        key: node.public_key,
        fingerprint: node.fingerprint.clone(),
    });
    save_pins(&pins)?;
    r.field("fingerprint", node.fingerprint);
    Ok(())
}

async fn board<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    verb: &Verb,
) -> Result<(), Refusal> {
    let text = address.to_string();
    let pins = load_pins(state)?;
    let link = link::pinned(env, &pins, &text, address).await?;
    let info = link::board_info(&link.node);
    if !info.reported {
        return Err(Refusal::new(
            1,
            "this board did not report what it offers, so its slots are not known",
        ));
    }
    if !info.offered {
        return Err(Refusal::new(1, SlotError::NoSlots.to_string()));
    }
    let slots = BoardSlots::new(
        link.handle.clone(),
        info.family.clone().unwrap_or_default(),
        true,
    );
    match verb {
        Verb::List => list(r, &slots).await,
        Verb::Confirm(id) => {
            slots.set_default(id, false).await?;
            r.say(format!("{id} is now the default slot"));
            r.field("default", id.clone());
            Ok(())
        }
        Verb::Remove(id) => {
            slots.remove(id, false).await?;
            r.say(format!("removed {id}"));
            r.field("removed", id.clone());
            Ok(())
        }
        Verb::Install(path) => install(r, state, &link, &info, path).await,
        Verb::Trial(id) => trial(r, env, state, &link, &slots, &pins, address, id).await,
        Verb::Fingerprint | Verb::Pin | Verb::Repin => Err(Refusal::new(2, "not a board command")),
    }
}

async fn list<W: Write>(r: &mut Report<'_, W>, slots: &BoardSlots) -> Result<(), Refusal> {
    let list = slots.list().await?;
    for line in list_lines(&list) {
        r.say(line);
    }
    let entries: Vec<_> = list
        .slots
        .iter()
        .map(|slot| {
            json!({
                "id": slot.id,
                "version": slot.version,
                "running": slot.id == list.running,
                "default": slot.id == list.default,
                "trial": list.trial.as_ref().is_some_and(|trial| trial.slot == slot.id),
                "starred": slot.pinned,
                "failed": slot.failed,
            })
        })
        .collect();
    r.field("running", list.running.clone());
    r.field("default", list.default.clone());
    r.field("keep", list.keep);
    r.field("slots", entries);
    Ok(())
}

/// The list as lines: a header, a trial line when one is pending, then one line per slot with
/// its state words.
fn list_lines(list: &SlotList) -> Vec<String> {
    let mut lines = vec![format!(
        "running {}, default {}, keep {}",
        list.running, list.default, list.keep
    )];
    if let Some(trial) = &list.trial {
        lines.push(format!("trial of {} is pending", trial.slot));
    }
    for slot in &list.slots {
        let mut words = Vec::new();
        if slot.id == list.running {
            words.push("running");
        }
        if slot.id == list.default {
            words.push("default");
        }
        if list
            .trial
            .as_ref()
            .is_some_and(|trial| trial.slot == slot.id)
        {
            words.push("trial");
        }
        if slot.pinned {
            words.push("starred");
        }
        if slot.failed {
            words.push("failed");
        }
        lines.push(format!(
            "{:<14} {:<24} {}",
            slot.id,
            words.join(" "),
            slot.version
        ));
    }
    lines
}

async fn install<W: Write>(
    r: &mut Report<'_, W>,
    state: &Path,
    link: &Link,
    info: &BoardInfo,
    path: &Path,
) -> Result<(), Refusal> {
    let Some(family) = info.family.clone() else {
        return Err(Refusal::new(
            1,
            "this board publishes no family, so a bundle cannot be checked against it",
        ));
    };
    if !path.is_file() {
        return Err(Refusal::new(
            1,
            format!("no such bundle file: {}", path.display()),
        ));
    }
    let device = match (&info.family, &info.serial) {
        (Some(family), Some(serial)) => {
            DeviceKey::board(family, serial).map_err(|error| Refusal::new(1, error.to_string()))?
        }
        _ => DeviceKey::node(link.node.fingerprint.clone())
            .map_err(|error| Refusal::new(1, error.to_string()))?,
    };
    let jobs = state.join("jobs");
    std::fs::create_dir_all(&jobs)
        .map_err(|error| Refusal::new(1, format!("{} cannot be made: {error}", jobs.display())))?;
    let installs = SlotInstalls::open(
        jobs.join("jobs.json"),
        Arc::clone(&link.sessions),
        Events::new(256),
        JobTiming::default(),
        DeviceHolds::new(),
        family.clone(),
        jobs.clone(),
    )
    .await
    .map_err(|error| Refusal::new(1, format!("the job list cannot be opened: {error}")))?;
    let job = installs
        .submit(
            device,
            &link.address,
            &family,
            true,
            InstallRequest {
                source: Source::Bundle(path.to_owned()),
                starred: false,
            },
        )
        .await?;
    let mut last = None;
    loop {
        let stage = installs.stage(job);
        if stage != last {
            // A final stage is said once, below, as the outcome; progress shows the steps before it.
            if let Some(stage) = stage.as_ref().filter(|stage| !stage.is_final()) {
                r.progress(stage.to_string());
            }
            last.clone_from(&stage);
        }
        match stage {
            Some(Stage::Done { slot }) => {
                r.say(format!("installed as {slot}"));
                r.field("slot", slot);
                return Ok(());
            }
            Some(Stage::Failed { reason }) => return Err(Refusal::new(1, reason)),
            Some(Stage::Cancelled) => return Err(Refusal::new(1, "the install was cancelled")),
            _ => tokio::time::sleep(POLL).await,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn trial<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    link: &Link,
    slots: &BoardSlots,
    pins: &Pins,
    address: &Address,
    id: &str,
) -> Result<(), Refusal> {
    let follower = Follower::open(state).with_recheck(env.recheck);
    let note = follower
        .start_trial(
            slots,
            &link.handle,
            TrialStart {
                id,
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: now_ms(),
            },
        )
        .await?;
    if let Some(note) = note {
        r.say(note);
    }
    r.progress(format!(
        "trial of {id} started; following it to its outcome"
    ));
    let key = link.node.public_key;
    let stop = async {
        match &env.stop {
            Some(stop) => stop.notified().await,
            None => std::future::pending().await,
        }
    };
    let ended = tokio::select! {
        ended = follower.follow(slots, &link.handle, key) => ended,
        () = stop => follower.stop(key),
    };
    let Some(ended) = ended else {
        return Err(Refusal::new(
            1,
            format!("the trial of {id} is not followed: no watch is held for the board"),
        ));
    };
    if let Err(why) = &ended.saved {
        r.say(format!(
            "note: the outcome is not saved, so the trial may be reported again after an Atlas restart: {why}"
        ));
    }
    trial_outcome(r, env, pins, address, id, ended.state).await
}

async fn trial_outcome<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    pins: &Pins,
    address: &Address,
    id: &str,
    state: TrialState,
) -> Result<(), Refusal> {
    match state {
        TrialState::Held(decided) => {
            r.say(format!(
                "came back in the trial slot {}: the trial held, confirmed by the board ({}). The board's reason: {}",
                decided.slot, decided.by, decided.reason
            ));
            r.field("outcome", "held");
            Ok(())
        }
        TrialState::DidNotHold(decided) => {
            r.field("outcome", "failed");
            Err(Refusal::new(
                1,
                format!(
                    "came back in the previous slot: the trial failed. It runs {}. The board's reason: {}",
                    running_words(&decided.running),
                    decided.reason
                ),
            ))
        }
        TrialState::Disagree {
            slot,
            record,
            running,
        } => Err(Refusal::new(
            1,
            format!(
                "did not decide: the board's record of the trial of {slot} says {record:?} and it runs {running}. Neither is picked."
            ),
        )),
        TrialState::NoRecord { slot, running } => Err(Refusal::new(
            1,
            format!(
                "did not decide: the board came back with no record of the trial of {slot}; it runs {}",
                running_words(&running)
            ),
        )),
        TrialState::Unreadable { slot, words } => Err(Refusal::new(
            1,
            format!(
                "did not decide: the board's records of the trial of {slot} cannot be read: {words}"
            ),
        )),
        TrialState::Replaced { .. } => {
            let text = address.to_string();
            let found = link::probe(env, address)
                .await
                .ok()
                .map(|node| node.fingerprint);
            match pins.get(&text) {
                Some(pin) => Err(link::replaced(&text, pin, found)),
                None => Err(Refusal::new(
                    3,
                    format!("the board at {text} was replaced during the trial; it is not pinned"),
                )),
            }
        }
        TrialState::Stopped { slot } => Err(Refusal::new(
            1,
            format!(
                "did not come back: you stopped following the trial of {slot}. The board's trial is unchanged."
            ),
        )),
        other => Err(Refusal::new(
            1,
            format!("did not come back: {other:?} (trial {id})"),
        )),
    }
}

fn running_words(running: &Running) -> String {
    match running {
        Running::Trial => "the trial slot".to_owned(),
        Running::Other(slot) => slot.clone(),
        Running::NotReported => "a slot the board did not name".to_owned(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

fn load_pins(state: &Path) -> Result<Pins, Refusal> {
    Pins::load(state).map_err(|why| Refusal::new(1, why))
}

fn save_pins(pins: &Pins) -> Result<(), Refusal> {
    pins.save().map_err(|why| Refusal::new(1, why))
}

async fn probe_or_refuse(env: &Env, address: &Address, text: &str) -> Result<Node, Refusal> {
    link::probe(env, address)
        .await
        .map_err(|why| Refusal::new(1, format!("the board at {text} did not answer: {why}")))
}
