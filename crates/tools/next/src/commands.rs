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
use orion::node::{Node, OperatorState};
use orion::{Address, ClientError};
use serde_json::json;

use crate::args::{Command, Verb};
use crate::identity;
use crate::link::{self, BoardInfo, Link, Security};
use crate::pins::{Pin, Pins};
use crate::report::Report;
use crate::{Env, parse_address};

/// How often an install's stage is looked at while it runs.
const POLL: Duration = Duration::from_millis(50);

/// The name this PC's identity is enrolled under on a board.
const ENROL_NAME: &str = "atlas-next";
/// The first line of a board command on a board with security off: its key is checked, but the
/// session is not signed.
pub const SECURITY_OFF_LINE: &str =
    "security off: the board's key is checked, the session is not signed";
/// Refused by `enrol` on a board with security off.
pub const SECURITY_OFF_NO_ENROL: &str =
    "this board runs with security off: there is nothing to enrol";

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
    let Some(path) = command
        .identity
        .clone()
        .or_else(|| env.identity_default.clone())
    else {
        r.say("no identity file: give --identity <file>");
        return r.finish(2);
    };
    let key = match identity::load_or_create(&path) {
        Ok(loaded) => loaded.secret,
        Err(why) => {
            r.say(why);
            return r.finish(1);
        }
    };
    // The session mode is asked once per run, and every command of this run uses the answer.
    let mode = link::security(&address).await;
    let outcome = match &command.verb {
        Verb::Fingerprint => fingerprint(&mut r, env, &state, &address, &key).await,
        Verb::Pin => pin(&mut r, env, &state, &address, &key).await,
        Verb::Repin => repin(&mut r, env, &state, &address, command.reflashed, &key).await,
        Verb::Enrol => enrol(&mut r, env, &state, &address, &key, mode).await,
        verb => board(&mut r, env, &state, &address, verb, &key, mode).await,
    };
    match outcome {
        Ok(()) => r.finish(0),
        Err(refusal) => {
            r.say(refusal.words.clone());
            if !matches!(
                command.verb,
                Verb::Fingerprint | Verb::Pin | Verb::Repin | Verb::Enrol
            ) && denied_by_board(&refusal.words)
            {
                r.say(format!(
                    "this PC's key is not enrolled on this board: run atlas-next enrol {}",
                    command.address
                ));
            }
            r.finish(refusal.code)
        }
    }
}

/// Whether a board's words are Orion's refusal of the call (`denied`): a key the board does not
/// know, or one it has not let act. Only the words reach here: a board command's refusal crosses
/// `atlas-board-slots` and `atlas-session` as text, so the variant (`ClientError::Refused`) is not
/// visible to the tool. `enrol` makes its call directly and matches the variant instead.
fn denied_by_board(words: &str) -> bool {
    words
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| word == orion::types::Error::Denied.as_str())
}

async fn fingerprint<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
    let node = probe_or_refuse(env, address, &text, key).await?;
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
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
    let node = probe_or_refuse(env, address, &text, key).await?;
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
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
    let node = probe_or_refuse(env, address, &text, key).await?;
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

/// `enrol`: asks the board to know this PC's identity. The request carries no proof, so the
/// board decides: its answer says whether this key is pending the owner's approval, enrolled, or
/// revoked. The request is made on a pinned board only (the pin rule), and never on a board with
/// security off, which has nothing to enrol.
async fn enrol<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    key: &[u8; 32],
    mode: Security,
) -> Result<(), Refusal> {
    let text = address.to_string();
    if mode == Security::Off {
        return Err(Refusal::new(1, SECURITY_OFF_NO_ENROL));
    }
    let pins = load_pins(state)?;
    let Some(pin) = pins.get(&text).cloned() else {
        let node = probe_or_refuse(env, address, &text, key).await?;
        return Err(Refusal::new(
            3,
            format!(
                "{text} has never been seen. Its key fingerprint is {}. Nothing is trusted yet: check the fingerprint, then run `atlas-next slots {text} pin`.",
                node.fingerprint
            ),
        ));
    };
    let public = identity::public_key(key);
    let fingerprint = identity::fingerprint(&public);
    let answer = link::enrol(address, pin.key, *key, ENROL_NAME)
        .await
        .map_err(|why| Refusal::new(1, format!("the enrolment did not finish: {why}")))?;
    let enrollment = match answer {
        Ok(enrollment) => enrollment,
        Err(orion::Error::Client(ClientError::OtherKey { answered, .. })) => {
            return Err(link::replaced(
                &text,
                &pin,
                Some(identity::fingerprint(&answered)),
            ));
        }
        Err(error) => return Err(Refusal::new(1, error.to_string())),
    };
    match enrollment.state {
        OperatorState::Pending => r.say(format!(
            "your key {fingerprint} is now pending on this board: the board's owner must approve it before this PC can do anything there"
        )),
        OperatorState::Enrolled => {
            r.say(format!("your key {fingerprint} is enrolled on this board"));
        }
        OperatorState::Revoked => {
            return Err(Refusal::new(
                1,
                format!(
                    "your key {fingerprint} is revoked on this board: a revoked key does not enrol again"
                ),
            ));
        }
        other => {
            return Err(Refusal::new(
                1,
                format!("the board answers your key {fingerprint} as {other:?}"),
            ));
        }
    }
    r.field("fingerprint", fingerprint);
    r.field("state", format!("{:?}", enrollment.state));
    Ok(())
}

async fn board<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    verb: &Verb,
    key: &[u8; 32],
    mode: Security,
) -> Result<(), Refusal> {
    let text = address.to_string();
    if mode == Security::Off {
        r.say(SECURITY_OFF_LINE);
    }
    let pins = load_pins(state)?;
    let link = link::pinned(env, &pins, &text, address, key).await?;
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
        Verb::Trial(id) => trial(r, env, state, &link, &slots, &pins, address, id, key).await,
        Verb::Fingerprint | Verb::Pin | Verb::Repin | Verb::Enrol => {
            Err(Refusal::new(2, "not a board command"))
        }
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
    key: &[u8; 32],
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
    let node_key = link.node.public_key;
    let stop = async {
        match &env.stop {
            Some(stop) => stop.notified().await,
            None => std::future::pending().await,
        }
    };
    let ended = tokio::select! {
        ended = follower.follow(slots, &link.handle, node_key) => ended,
        () = stop => follower.stop(node_key),
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
    trial_outcome(r, env, pins, address, id, ended.state, key).await
}

async fn trial_outcome<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    pins: &Pins,
    address: &Address,
    id: &str,
    state: TrialState,
    key: &[u8; 32],
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
            let found = link::probe(env, address, key)
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

async fn probe_or_refuse(
    env: &Env,
    address: &Address,
    text: &str,
    key: &[u8; 32],
) -> Result<Node, Refusal> {
    link::probe(env, address, key)
        .await
        .map_err(|why| Refusal::new(1, format!("the board at {text} did not answer: {why}")))
}

#[cfg(test)]
mod tests {
    use super::denied_by_board;

    #[test]
    fn only_orions_denied_word_marks_a_refusal_of_the_board() {
        assert!(denied_by_board("the call failed: denied"));
        assert!(denied_by_board("denied"));
        assert!(!denied_by_board("the board did not answer: timed out"));
        assert!(!denied_by_board("undenied"));
    }
}
