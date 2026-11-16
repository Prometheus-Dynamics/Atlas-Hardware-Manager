//! The commands. Each one either says what happened and succeeds, or refuses with a status and
//! the words to show. The words of the board, the install and the trial are passed on as they are.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atlas_actions::{BUSY_CODE, BoardActions};
use atlas_board_slots::trial_updates::read as read_trial_updates;
use atlas_board_slots::{
    BoardSlots, Follower, InstallRequest, SlotError, SlotInstalls, Source, TrialStart, TrialUpdates,
};
use atlas_jobs::{DeviceHolds, Events, JobTiming};
use atlas_session::Failure;
use atlas_slots::{SlotList, Stage};
use atlas_trial::{Running, TrialState};
use atlas_types::DeviceKey;
use orion::node::{Node, OperatorState};
use orion::{Address, ClientError};
use serde_json::json;

use crate::args::{Command, Verb};
use crate::identity;
use crate::link::{self, BoardInfo, Link};
use crate::pins::{Pin, Pins};
use crate::report::Report;
use crate::{Env, parse_address};

/// How often an install's stage is looked at while it runs.
const POLL: Duration = Duration::from_millis(50);

/// The name this PC's identity is enrolled under on a board.
const ENROL_NAME: &str = "atlas-next";

/// A command that did not succeed: its status and the words to show. `denied` is set when the
/// board's refusal was Orion's `Denied` (a key the board does not know, or one it has not let
/// act): it is read from the kind the session carried, never from the words.
#[derive(Debug)]
pub struct Refusal {
    pub code: i32,
    pub words: String,
    pub denied: bool,
    /// The board refused it as busy: its run ended Rejected with BUSY (typed from the status code).
    pub busy: bool,
}

impl Refusal {
    pub fn new(code: i32, words: impl Into<String>) -> Self {
        Self {
            code,
            words: words.into(),
            denied: false,
            busy: false,
        }
    }
}

impl From<SlotError> for Refusal {
    fn from(error: SlotError) -> Self {
        let denied = error.failure().is_some_and(Failure::is_denied);
        let busy = matches!(&error, SlotError::Board(board) if board.is_busy());
        Self {
            code: 1,
            words: error.to_string(),
            denied,
            busy,
        }
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
    let outcome = match &command.verb {
        Verb::Fingerprint => fingerprint(&mut r, &state, &address, &key).await,
        Verb::Pin => pin(&mut r, &state, &address, &key).await,
        Verb::Repin => repin(&mut r, &state, &address, command.reflashed, &key).await,
        Verb::Enrol => enrol(&mut r, env, &state, &address, &key).await,
        verb => board(&mut r, env, &state, &address, verb, &key).await,
    };
    match outcome {
        Ok(()) => r.finish(0),
        Err(refusal) => {
            r.say(refusal.words.clone());
            if !matches!(
                command.verb,
                Verb::Fingerprint | Verb::Pin | Verb::Repin | Verb::Enrol
            ) && refusal.denied
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

async fn fingerprint<W: Write>(
    r: &mut Report<'_, W>,
    state: &Path,
    address: &Address,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
    let proved = transport_or_refuse(address, &text, key).await?;
    let fingerprint = identity::fingerprint(&proved);
    r.say(format!("{text}: key fingerprint {fingerprint}"));
    let pins = load_pins(state)?;
    match pins.get(&text) {
        None => r.say("not pinned: run `pin` to trust this key"),
        Some(pin) if pin.key == proved => {
            r.say("pinned: this is the key the address is pinned to");
        }
        Some(pin) => r.say(format!(
            "not the pinned key: pinned as {}; a reflash is the one legitimate cause",
            pin.fingerprint
        )),
    }
    r.field("address", text);
    r.field("fingerprint", fingerprint);
    Ok(())
}

async fn pin<W: Write>(
    r: &mut Report<'_, W>,
    state: &Path,
    address: &Address,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
    let proved = transport_or_refuse(address, &text, key).await?;
    let fingerprint = identity::fingerprint(&proved);
    let mut pins = load_pins(state)?;
    match pins.get(&text).cloned() {
        Some(pin) if pin.key == proved => {
            r.say(format!("already pinned: {fingerprint}"));
        }
        Some(pin) => return Err(link::replaced(&text, &pin, Some(fingerprint.clone()))),
        None => {
            pins.set(Pin {
                address: text.clone(),
                key: proved,
                fingerprint: fingerprint.clone(),
            });
            save_pins(&pins)?;
            r.say(format!(
                "pinned {fingerprint} for {text}: every command now checks this key"
            ));
        }
    }
    r.field("fingerprint", fingerprint);
    Ok(())
}

async fn repin<W: Write>(
    r: &mut Report<'_, W>,
    state: &Path,
    address: &Address,
    reflashed: bool,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
    let proved = transport_or_refuse(address, &text, key).await?;
    let fingerprint = identity::fingerprint(&proved);
    let mut pins = load_pins(state)?;
    let old = pins.get(&text).cloned();
    if !reflashed {
        let pinned = old
            .as_ref()
            .map_or_else(|| "none".to_owned(), |pin| pin.fingerprint.clone());
        return Err(Refusal::new(
            3,
            format!(
                "repin replaces the pin, and is for a reflashed board only. Pinned: {pinned}. Answering now: {fingerprint}. Nothing was changed. If the board was reflashed, run it again with --reflashed."
            ),
        ));
    }
    match &old {
        Some(pin) if pin.key == proved => {
            r.say(format!("unchanged: {fingerprint} is still the pin"));
        }
        Some(pin) => r.say(format!(
            "repinned {text}: {} -> {fingerprint}",
            pin.fingerprint
        )),
        None => r.say(format!("pinned {text} as {fingerprint} (it had no pin)")),
    }
    pins.set(Pin {
        address: text,
        key: proved,
        fingerprint: fingerprint.clone(),
    });
    save_pins(&pins)?;
    r.field("fingerprint", fingerprint);
    Ok(())
}

/// The key the board proves it holds (see [`link::transport_key`]), or the words for a board that
/// did not answer.
async fn transport_or_refuse(
    address: &Address,
    text: &str,
    key: &[u8; 32],
) -> Result<[u8; 32], Refusal> {
    link::transport_key(address, key)
        .await
        .map_err(|why| Refusal::new(1, format!("the board at {text} did not answer: {why}")))
}

/// `enrol`: asks the board to know this PC's identity. The request carries no proof, so the
/// board decides: its answer says whether this key is pending the owner's approval, enrolled, or
/// revoked. The request is made on a pinned board only (the pin rule).
async fn enrol<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let text = address.to_string();
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
) -> Result<(), Refusal> {
    let text = address.to_string();
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
        Verb::List => list(r, &slots, &link).await,
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
        Verb::Install(path) => install(r, state, &link, &info, path, env.wait)
            .await
            .map(drop),
        Verb::Trial(id) => trial(r, env, state, &link, &slots, &pins, address, id, key).await,
        Verb::Fingerprint | Verb::Pin | Verb::Repin | Verb::Enrol => {
            Err(Refusal::new(2, "not a board command"))
        }
    }
}

async fn list<W: Write>(
    r: &mut Report<'_, W>,
    slots: &BoardSlots,
    link: &Link,
) -> Result<(), Refusal> {
    let list = slots.list().await?;
    for line in list_lines(&list) {
        r.say(line);
    }
    // A board that states a trial block says so where the list is read; nothing is shown when no block is stated.
    let updates = trial_updates_of(link).await;
    if let Some(line) = trial_block_line(&updates) {
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

/// The board's trial updates, read now through the shared reading. A read that fails says nothing.
async fn trial_updates_of(link: &Link) -> TrialUpdates {
    read_trial_updates(&BoardActions::new(link.handle.clone())).await
}

/// The line a user reads when the board states a trial block; `None` when it states none (or cannot say).
fn trial_block_line(updates: &TrialUpdates) -> Option<String> {
    updates.blocks_trials().then(|| updates.words())
}

/// The refusal for a trial on a board that states a block: the typed reason, before anything is written.
/// `None` when the trial may be started.
fn trial_refusal(updates: &TrialUpdates) -> Option<String> {
    trial_block_line(updates)
}

/// The list as lines: a header, a trial line when one is pending, then one line per slot with
/// its state words.
pub(crate) fn list_lines(list: &SlotList) -> Vec<String> {
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

/// Installs the bundle at `path` on the board and returns the id it was installed as. Shared with
/// `cycle`, which needs the id.
/// The exit status of an install whose outcome is still unknown after a reconnect at the wait's
/// bound: not a failure, and not a success. The job stays unknown; running it again is the user's
/// choice.
pub const UNKNOWN_EXIT: i32 = 4;

pub(crate) async fn install<W: Write>(
    r: &mut Report<'_, W>,
    state: &Path,
    link: &Link,
    info: &BoardInfo,
    path: &Path,
    wait: Duration,
) -> Result<String, Refusal> {
    // Lemnos BB1c: an install still writes the bay; only the trial is refused. Say so before writing.
    if let Some(line) = trial_block_line(&trial_updates_of(link).await) {
        r.say(line);
        r.say("the slot is installed, but the boot loader will not start it as a trial until the board takes trial updates");
    }
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
    // When the install was first seen unknown. Bounded by the wait, like every wait of the tool.
    let mut unknown_since: Option<std::time::Instant> = None;
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
                r.field("slot", slot.clone());
                return Ok(slot);
            }
            Some(Stage::Failed { reason }) => {
                // The run's typed failure keeps the board's status code: BUSY is read from it.
                let busy = installs
                    .failure(job)
                    .is_some_and(|failure| failure.code() == Some(BUSY_CODE));
                return Err(Refusal {
                    busy,
                    ..Refusal::new(1, reason)
                });
            }
            Some(Stage::Cancelled) => return Err(Refusal::new(1, "the install was cancelled")),
            Some(Stage::Unknown { reason, checking }) => {
                let since = *unknown_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() >= wait {
                    return Err(Refusal::new(
                        UNKNOWN_EXIT,
                        format!(
                            "the install's outcome is unknown after a reconnect: {reason}. \
                             Checked: {checking}. The job stays unknown; running it again is your choice."
                        ),
                    ));
                }
                tokio::time::sleep(POLL).await;
            }
            _ => {
                unknown_since = None;
                tokio::time::sleep(POLL).await;
            }
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
    // A board that states a trial block refuses the trial before anything is written, with the typed reason.
    if let Some(words) = trial_refusal(&trial_updates_of(link).await) {
        return Err(Refusal::new(1, words));
    }
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

pub(crate) async fn trial_outcome<W: Write>(
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

pub(crate) fn load_pins(state: &Path) -> Result<Pins, Refusal> {
    Pins::load(state).map_err(|why| Refusal::new(1, why))
}

pub(crate) fn save_pins(pins: &Pins) -> Result<(), Refusal> {
    pins.save().map_err(|why| Refusal::new(1, why))
}

pub(crate) async fn probe_or_refuse(
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
    use atlas_actions::ActionError;
    use atlas_board_slots::SlotError;
    use atlas_session::{Failure, FailureKind};

    use super::Refusal;

    #[test]
    fn a_denied_refusal_marks_the_not_enrolled_line_whatever_its_words() {
        // Reworded on purpose: the kind decides.
        let error = SlotError::Board(ActionError::Refused(Failure::new(
            FailureKind::Denied,
            None,
            "no",
        )));
        assert!(Refusal::from(error).denied);
        let unknown = SlotError::Board(ActionError::Unknown {
            name: "system.slots.list".into(),
            reason: Failure::new(FailureKind::Denied, Some(10), "no"),
        });
        assert!(Refusal::from(unknown).denied);
    }

    #[test]
    fn a_refusal_that_is_not_denied_does_not_mark_it_even_if_its_words_say_denied() {
        let board_words = SlotError::Board(ActionError::Refused("denied".into()));
        assert!(!Refusal::from(board_words).denied);
        let other = SlotError::Board(ActionError::Failed(Failure::new(
            FailureKind::Other,
            None,
            "the call failed: denied",
        )));
        assert!(!Refusal::from(other).denied);
        let refused = SlotError::Board(ActionError::Refused(Failure::new(
            FailureKind::Refused,
            Some(11),
            "busy",
        )));
        assert!(!Refusal::from(refused).denied);
    }
}

#[cfg(test)]
mod trial_refusal_tests {
    use super::*;

    #[test]
    fn a_blocked_board_refuses_the_trial_with_its_typed_reason_and_a_clear_one_does_not() {
        let blocked = TrialUpdates::Blocked {
            block: atlas_board_slots::TrialBlock::LoaderNotUpdated,
            text: String::new(),
            detail: String::new(),
        };
        assert_eq!(
            trial_refusal(&blocked).as_deref(),
            Some("this board cannot take trial updates: loader not updated"),
            "the refusal names the typed reason, before anything is written"
        );
        assert_eq!(trial_refusal(&TrialUpdates::Clear), None);
        assert_eq!(trial_refusal(&TrialUpdates::NotReported), None);
    }

    #[test]
    fn an_unknown_block_is_refused_and_shown_as_sent() {
        let unknown = TrialUpdates::Blocked {
            block: atlas_board_slots::TrialBlock::from_wire("new-reason"),
            text: String::new(),
            detail: String::new(),
        };
        let words = trial_refusal(&unknown).expect("an unknown block still refuses");
        assert!(words.contains("unknown (as sent: new-reason)"), "{words}");
    }
}
