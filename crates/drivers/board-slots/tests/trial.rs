//! The trial watch on a simulated board, through its real session: the states a person sees, read
//! from the simulated board's own `trial` and `resolved` records; several power cycles before the
//! outcome; the record and the running version that disagree; the device list's settling predicate
//! and health; persistence across an Atlas restart; a board that stays away and is stopped by a
//! person; and a board replaced mid-trial.
//!
//! Time is the simulated slot clock (`slot_clock_advance`), never the wall clock. The only real
//! waits are the watch's own poll of a settling trial (set short in these tests) and until-checks
//! on state that the board changes at once.

mod common;

use std::sync::Arc;
use std::time::Duration;

use atlas_board_slots::{Ended, Follower, InstallRequest, Source, TrialStart};
use atlas_inventory::Health;
use atlas_session::Presence;
use atlas_sim_node::{DEADLINE, SimNode, call, names};
use atlas_slots::{Resolution, Stage};
use atlas_trial::{Outcome, Running, Seen, TrialState, health_while_settling, hex, settling};
use common::{BOARD, Rig, bundle_bytes, device, until, write_bundle};

/// The watch's own poll of a settling trial in these tests: short, so states show quickly.
const SHORT: Duration = Duration::from_millis(20);
/// A poll that does not fire during a test: outcomes are then read on the board's events only.
const LONG: Duration = Duration::from_secs(3600);

fn key(rig: &Rig) -> [u8; 32] {
    rig.handle.node_key().expect("the board is connected")
}

fn follower(rig: &Rig, recheck: Duration) -> Arc<Follower> {
    Arc::new(Follower::open(&rig.dir.join("state")).with_recheck(recheck))
}

/// Installs `id` at `version` through the job path, and waits for it to finish.
async fn install(rig: &Rig, id: &str, version: &str) {
    let path = write_bundle(
        &rig.dir.join("bundles"),
        &format!("{id}.tar"),
        &bundle_bytes(id, version, None),
    );
    let job = rig
        .installs
        .submit(
            device(),
            &rig.address(),
            BOARD,
            true,
            InstallRequest {
                source: Source::Bundle(path),
                starred: false,
            },
        )
        .await
        .expect("the install is queued");
    until("the install to finish", || {
        rig.installs
            .stage(job)
            .is_some_and(|stage| stage.is_final())
    })
    .await;
    assert!(
        matches!(rig.installs.stage(job), Some(Stage::Done { .. })),
        "the install ends with a slot: {:?}",
        rig.installs.stage(job)
    );
}

/// Follows the watch of this board on its own task. The follow ends with the outcome.
fn spawn_follow(follower: &Arc<Follower>, rig: &Rig) -> tokio::task::JoinHandle<Option<Ended>> {
    let follower = Arc::clone(follower);
    let slots = rig.slots.clone();
    let handle = rig.handle.clone();
    let board = key(rig);
    tokio::spawn(async move { follower.follow(&slots, &handle, board).await })
}

/// Waits until some watch state satisfies `check`.
async fn until_state(follower: &Follower, what: &str, check: impl Fn(&TrialState) -> bool) {
    until(what, || follower.states().iter().any(&check)).await;
}

fn settling_with(attempts: usize) -> impl Fn(&TrialState) -> bool {
    move |state| matches!(state, TrialState::Settling(s) if s.attempts.len() == attempts)
}

/// The board leaves: its address closes the session, and nothing answers until it comes back.
fn leave(rig: &Rig) {
    rig.relay.away(true);
    rig.relay.cut();
}

/// The board comes back as the same node.
fn come_back(rig: &Rig) {
    rig.relay.away(false);
}

/// A power cycle of the board: its boot runs the pending trial again (no attempt is spent when the
/// boot reached the daemon before the cut).
fn power_cycle_model(rig: &Rig) {
    let mut model = rig.node.model();
    model.slots.power_cut();
    model.slots.reboot();
}

/// Waits until a new connection to the board says `slot` is running (its published identity
/// follows the boot). The identity is what the watch reads on its next return.
async fn identity_says(rig: &Rig, slot: &str) {
    until("the board to publish its running slot", || {
        rig.node
            .connect("probe")
            .ok()
            .and_then(|orion| orion.node().ok())
            .is_some_and(|node| {
                node.facts
                    .iter()
                    .any(|fact| fact.name == "version.slot" && fact.value == slot.as_bytes())
            })
    })
    .await;
}

fn ended_state(ended: Option<Ended>) -> TrialState {
    let ended = ended.expect("the watch ended with an outcome");
    assert!(
        ended.saved.is_ok(),
        "the released watch was saved: {:?}",
        ended.saved
    );
    ended.state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_trial_is_followed_across_power_cycles_and_reported_once() {
    let rig = Rig::start("held").await;
    install(&rig, "slot-2", "2.0.0").await;
    let follower = follower(&rig, SHORT);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let task = spawn_follow(&follower, &rig);
    until_state(&follower, "the trial to settle", settling_with(1)).await;

    // The device list: settling beside its name, and its health is not red.
    let list = rig.slots.list().await.expect("the board lists its slots");
    let seen = Seen::Found {
        key: key(&rig),
        running: Some("slot-2".into()),
    };
    assert!(settling(Some(&list), &seen));
    assert_eq!(
        health_while_settling(Health::Fault, settling(Some(&list), &seen)),
        Health::Away
    );

    // Power cycles during the trial: each one leaves, shows the watch as not back yet, and the
    // board's own record counts the boot.
    for attempts in [2, 3] {
        leave(&rig);
        until_state(&follower, "the board to be away", |state| {
            matches!(state, TrialState::NotBackYet { .. })
        })
        .await;
        power_cycle_model(&rig);
        identity_says(&rig, "slot-2").await;
        come_back(&rig);
        until_state(
            &follower,
            "the board to settle again",
            settling_with(attempts),
        )
        .await;
    }

    // The board confirms itself once its daemon has been up long enough.
    rig.node.slot_clock_advance(30);
    let state = ended_state(task.await.expect("the follow task ends"));
    let TrialState::Held(decided) = state else {
        panic!("expected held, got {state:?}");
    };
    assert_eq!(decided.by, "auto");
    assert_eq!(decided.running, Running::Trial);
    assert!(!decided.reason.is_empty(), "the board's reason is kept");
    assert!(follower.states().is_empty(), "the outcome is released");

    // It is not reported again after an Atlas restart, and the board's default is the new slot.
    assert!(Follower::open(&rig.dir.join("state")).states().is_empty());
    let list = rig.slots.list().await.expect("lists");
    assert_eq!(list.default, "slot-2");
    assert!(list.trial.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_record_and_a_running_version_that_disagree_are_both_shown_and_neither_is_picked() {
    let rig = Rig::start("disagree").await;
    install(&rig, "slot-2", "2.0.0").await;
    let follower = follower(&rig, LONG);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let task = spawn_follow(&follower, &rig);
    until_state(&follower, "the trial to settle", settling_with(1)).await;

    // The board confirms, but its identity says it runs the old slot.
    rig.node.slot_clock_advance(30);
    rig.node.model().slots.active = Some("slot-1".into());
    identity_says(&rig, "slot-1").await;
    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    come_back(&rig);

    let state = ended_state(task.await.expect("the follow task ends"));
    assert_eq!(
        state,
        TrialState::Disagree {
            slot: "slot-2".into(),
            record: Outcome::Confirmed,
            running: "slot-1".into(),
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trial_that_does_not_hold_says_the_boards_reason_and_the_version_it_runs() {
    let rig = Rig::start("fell-back").await;
    install(&rig, "slot-2", "2.0.0").await;
    rig.node.slot_set_healthy(false);
    let follower = follower(&rig, LONG);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let task = spawn_follow(&follower, &rig);
    until_state(&follower, "the trial to settle", settling_with(1)).await;

    // The deadline passes with the board not healthy: it restarts into the default.
    rig.node.slot_clock_advance(600);
    identity_says(&rig, "slot-1").await;
    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    come_back(&rig);

    let state = ended_state(task.await.expect("the follow task ends"));
    let TrialState::DidNotHold(decided) = state else {
        panic!("expected did not hold, got {state:?}");
    };
    assert!(decided.reason.contains("not healthy"), "{}", decided.reason);
    assert_eq!(decided.running, Running::Other("slot-1".into()));
    assert_eq!(decided.by, "auto");
    let list = rig.slots.list().await.expect("lists");
    assert!(list.get("slot-2").is_some_and(|slot| slot.failed));
    assert_eq!(
        list.resolved.map(|resolved| resolved.outcome),
        Some(Resolution::FellBack)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_boots_that_never_reach_the_daemon_fall_back_and_the_watch_says_why() {
    let rig = Rig::start("three-boots").await;
    install(&rig, "slot-2", "2.0.0").await;
    rig.node.slot_boots_without_daemon(3);
    let follower = follower(&rig, LONG);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let task = spawn_follow(&follower, &rig);
    // The first boot never reached the daemon: the board runs the default, with two attempts left.
    until_state(&follower, "the first boot to be recorded", |state| {
        matches!(state, TrialState::Settling(s)
            if s.attempts_left == Some(2) && s.running == Running::Other("slot-1".into()))
    })
    .await;

    // Two more boots that never reach the daemon. The second one spends the last attempt, and
    // the board's return after it is where the outcome is read.
    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    power_cycle_model(&rig);
    come_back(&rig);
    until_state(
        &follower,
        "the second boot to be recorded",
        |state| matches!(state, TrialState::Settling(s) if s.attempts_left == Some(1)),
    )
    .await;
    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    power_cycle_model(&rig);
    identity_says(&rig, "slot-1").await;
    come_back(&rig);

    let state = ended_state(task.await.expect("the follow task ends"));
    let TrialState::DidNotHold(decided) = state else {
        panic!("expected did not hold, got {state:?}");
    };
    assert_eq!(
        decided.reason,
        "the trial slot did not reach a running daemon in 3 boots"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unwritable_state_dir_is_shown_as_a_record_error_and_never_as_a_guessed_outcome() {
    let rig = Rig::start("record-error").await;
    install(&rig, "slot-2", "2.0.0").await;
    rig.node.slot_fail_record_writes(true);
    let follower = follower(&rig, LONG);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let task = spawn_follow(&follower, &rig);
    until_state(
        &follower,
        "the record error to show",
        |state| matches!(state, TrialState::Settling(s) if s.record_error.is_some()),
    )
    .await;

    // The board's fall back keeps the reason: the record could not be written.
    rig.node.slot_clock_advance(600);
    identity_says(&rig, "slot-1").await;
    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    come_back(&rig);

    let state = ended_state(task.await.expect("the follow task ends"));
    let TrialState::DidNotHold(decided) = state else {
        panic!("expected did not hold, got {state:?}");
    };
    assert!(
        decided.reason.contains("could not be written"),
        "{}",
        decided.reason
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_explicit_confirm_by_another_caller_is_reported_as_a_call() {
    let rig = Rig::start("call").await;
    install(&rig, "slot-2", "2.0.0").await;
    let follower = follower(&rig, LONG);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let task = spawn_follow(&follower, &rig);
    until_state(&follower, "the trial to settle", settling_with(1)).await;

    let other = rig
        .node
        .connect("another-caller")
        .expect("another caller connects");
    let finished = call(&other, names::SLOTS_CONFIRM, &[]).expect("the confirm is answered");
    assert_eq!(
        finished.outcome,
        orion::actions::Outcome::Succeeded,
        "the confirm is done: {}",
        finished.detail
    );

    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    come_back(&rig);

    let state = ended_state(task.await.expect("the follow task ends"));
    let TrialState::Held(decided) = state else {
        panic!("expected held, got {state:?}");
    };
    assert_eq!(decided.by, "call");
    assert!(!decided.caller.is_empty(), "the caller is named");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trial_is_followed_after_atlas_restarts_and_its_outcome_is_reported_once() {
    let rig = Rig::start("restart").await;
    install(&rig, "slot-2", "2.0.0").await;
    let before = follower(&rig, LONG);
    before
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    // Atlas closes; the board runs on, confirms itself, and comes back while Atlas is closed.
    drop(before);
    rig.node.slot_clock_advance(30);
    leave(&rig);
    come_back(&rig);

    let after = follower(&rig, LONG);
    assert!(matches!(
        after.states().as_slice(),
        [TrialState::NotBackYet { slot }] if slot == "slot-2"
    ));
    let state = ended_state(
        spawn_follow(&after, &rig)
            .await
            .expect("the follow task ends"),
    );
    let TrialState::Held(decided) = state else {
        panic!("expected held after the restart, got {state:?}");
    };
    assert_eq!(decided.by, "auto");

    // Reported once: the next start has nothing to report.
    let next = follower(&rig, LONG);
    assert!(next.states().is_empty());
    assert!(
        spawn_follow(&next, &rig)
            .await
            .expect("the task joins")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_board_that_never_returns_keeps_its_watch_and_a_person_can_stop_it() {
    let rig = Rig::start("stays-away").await;
    install(&rig, "slot-2", "2.0.0").await;
    let follower = follower(&rig, SHORT);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let board = key(&rig);
    let task = spawn_follow(&follower, &rig);
    until_state(&follower, "the trial to settle", settling_with(1)).await;

    leave(&rig);
    until_state(&follower, "the board to be away", |state| {
        matches!(state, TrialState::NotBackYet { .. })
    })
    .await;
    // Nothing gives up on its own: the watch stays while the board stays away.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!task.is_finished(), "the watch is still followed");
    assert!(matches!(
        follower.states().as_slice(),
        [TrialState::NotBackYet { .. }]
    ));

    // A person stops it. Nothing on the board changes.
    let stopped = ended_state(follower.stop(board));
    assert!(matches!(stopped, TrialState::Stopped { ref slot } if slot == "slot-2"));
    assert!(task.await.expect("the follow task ends").is_none());
    assert!(follower.states().is_empty());

    come_back(&rig);
    until("the board to be back", || rig.handle.node_key().is_some()).await;
    let list = rig.slots.list().await.expect("lists");
    assert!(
        list.trial.is_some_and(|trial| trial.slot == "slot-2"),
        "the board's trial is still pending: Atlas did not confirm it"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_board_replaced_mid_trial_ends_the_watch_and_reads_nothing_from_the_other() {
    let rig = Rig::start("replaced").await;
    install(&rig, "slot-2", "2.0.0").await;
    let follower = follower(&rig, SHORT);
    follower
        .start_trial(
            &rig.slots,
            &rig.handle,
            TrialStart {
                id: "slot-2",
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms: 1,
            },
        )
        .await
        .expect("the trial starts");
    let board = key(&rig);
    let task = spawn_follow(&follower, &rig);
    until_state(&follower, "the trial to settle", settling_with(1)).await;

    // Another board answers at the same address: a new key, a new directory, the same port.
    let port = rig.node.network().port();
    let dir = rig.dir.clone();
    let Rig { node, relay, .. } = rig;
    drop(node);
    let replacement = SimNode::start_with(&dir.join("replacement"), &common::kind(), Some(port))
        .expect("the replacement node starts on the same port");
    let _relay = relay;

    let ended = tokio::time::timeout(DEADLINE, task)
        .await
        .expect("the watch ends")
        .expect("the follow task joins");
    let state = ended_state(ended);
    let TrialState::Replaced { expected, found } = state else {
        panic!("expected replaced, got {state:?}");
    };
    assert_eq!(expected, hex(&board));
    // The session names the other node's key only where the locked Orion revision says it
    // (`board/session`, LOCK): when it does not, `found` is unknown, never the trial's own key.
    assert!(found.as_ref().is_none_or(|found| *found != expected));
    assert_eq!(
        replacement.runs_started(names::SLOTS_LIST),
        0,
        "nothing was read from the other board"
    );
    assert!(follower.states().is_empty());
}

/// The session is pinned to the board's key, so a replacement is seen as a replacement and not as
/// the same board coming back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_session_reports_a_replacement_as_replaced() {
    let rig = Rig::start("pinned").await;
    let port = rig.node.network().port();
    let dir = rig.dir.clone();
    let Rig {
        node,
        relay,
        handle,
        ..
    } = rig;
    drop(node);
    let _replacement = SimNode::start_with(&dir.join("replacement"), &common::kind(), Some(port))
        .expect("the replacement starts");
    let _relay = relay;
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Replaced { .. })),
    )
    .await
    .expect("the session reports the replacement")
    .expect("the monitor runs");
}
