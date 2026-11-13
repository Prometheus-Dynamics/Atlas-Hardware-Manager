//! A board's slots through its simulated Orion node: every slot operation, the install end to end,
//! lost sessions during the push and during the board's install step, cancel, the wrong-board
//! refusal before any byte is sent, the trial's record, the guarded removal, a board with no slots
//! contract, and a list refused for size.

mod common;

use atlas_actions::CancelState;
use atlas_actions::{Action, BoardActions, Reported};
use atlas_board_slots::{BoardSlots, InstallRequest, SlotError, Source};
use atlas_image::BoardOutcome;
use atlas_jobs::JobId;
use atlas_sim_node::names;
use atlas_slots::{RemoveRefusal, Resolution, SlotList, Stage};
use common::{BOARD, Rig, bundle_bytes, device, until, write_bundle};
use std::time::Duration;

/// The install request for a bundle file.
fn request(path: std::path::PathBuf, starred: bool) -> InstallRequest {
    InstallRequest {
        source: Source::Bundle(path),
        starred,
    }
}

async fn list(rig: &Rig) -> SlotList {
    rig.slots.list().await.expect("the board lists its slots")
}

/// Waits until the install's stage satisfies `ready`.
async fn stage_is(rig: &Rig, job: JobId, what: &str, ready: impl Fn(&Stage) -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(20), async {
        while !rig.installs.stage(job).is_some_and(|stage| ready(&stage)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    if waited.is_err() {
        panic!(
            "timed out waiting for {what}: stage {:?}, job {:?}",
            rig.installs.stage(job),
            rig.installs.jobs().get(job)
        );
    }
}

/// Waits for an install to end with a slot (returns the slot id).
async fn installed(rig: &Rig, job: JobId) -> String {
    stage_is(rig, job, "the install to finish", |stage| stage.is_final()).await;
    match rig.installs.stage(job) {
        Some(Stage::Done { slot }) => slot,
        other => panic!("the install did not finish with a slot: {other:?}"),
    }
}

/// Installs `id` at `version` through the job path and returns its slot id.
async fn install_bundle(rig: &Rig, id: &str, version: &str) -> String {
    let path = write_bundle(
        &rig.dir.join("bundles"),
        &format!("{id}.tar"),
        &bundle_bytes(id, version, None),
    );
    let job = rig
        .installs
        .submit(device(), &rig.address(), BOARD, true, request(path, false))
        .await
        .expect("the install is queued");
    installed(rig, job).await
}

fn reboot_action() -> Action {
    Action {
        name: names::REBOOT.into(),
        label: Reported::NotReported,
        description: Reported::NotReported,
        args: Reported::Known(Vec::new()),
        long_running: Reported::Known(false),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_reads_the_board_slots_with_the_signature_per_slot() {
    let rig = Rig::start("list").await;
    let listed = list(&rig).await;
    assert_eq!(listed.running, "slot-1");
    assert_eq!(listed.default, "slot-1");
    assert!(listed.trial.is_none() && listed.resolved.is_none());
    let slot = listed.get("slot-1").expect("slot-1 is listed");
    assert!(slot.has_flag("running") && slot.has_flag("default"));
    assert_eq!(
        slot.signature,
        atlas_slots::Reported::Known(BoardOutcome::NotCheckedUnsigned)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn install_end_to_end_from_a_bundle_ends_with_the_new_slot() {
    let rig = Rig::start("install").await;
    let id = install_bundle(&rig, "slot-2", "2.0.0").await;
    assert_eq!(id, "slot-2");
    let listed = list(&rig).await;
    let slot = listed.get("slot-2").expect("slot-2 is listed");
    assert_eq!(slot.good, "untried");
    assert_eq!(slot.version, "2.0.0");
    assert_eq!(rig.node.runs_started(names::SLOTS_INSTALL_FILE), 1);
    assert_eq!(rig.node.runs_started(names::SLOTS_ACCEPT), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_for_another_board_is_refused_before_any_byte_is_sent() {
    let rig = Rig::start("wrong-board").await;
    let path = write_bundle(
        &rig.dir.join("bundles"),
        "other.tar",
        &bundle_bytes("slot-9", "9.0.0", Some("other-board")),
    );
    let job = rig
        .installs
        .submit(device(), &rig.address(), BOARD, true, request(path, false))
        .await
        .expect("the job is queued");
    stage_is(&rig, job, "the refusal", |stage| stage.is_final()).await;
    match rig.installs.stage(job) {
        Some(Stage::Failed { reason }) => {
            assert!(
                reason.contains("other-board") && reason.contains(BOARD),
                "{reason}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(
        rig.node.runs_started(names::SLOTS_ACCEPT),
        0,
        "no accept was sent"
    );
    assert_eq!(
        rig.node.runs_started(names::SLOTS_INSTALL_FILE),
        0,
        "no install was sent"
    );
    assert!(
        rig.installs.push_counters(job).is_none(),
        "no push was started"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_lost_during_the_push_resumes_without_a_second_install() {
    let rig = Rig::start("lost-push").await;
    // A bundle big enough that the push is still moving when the link is cut.
    let big = bundle_bytes("slot-2", "2.0.0", None).repeat(64);
    let size = big.len() as u64;
    let path = write_bundle(&rig.dir.join("bundles"), "big.tar", &big);
    let job = rig
        .installs
        .submit(device(), &rig.address(), BOARD, true, request(path, false))
        .await
        .expect("queued");
    stage_is(
        &rig,
        job,
        "bytes moving",
        |stage| matches!(stage, Stage::Sending { sent, .. } if *sent > 0),
    )
    .await;
    rig.relay.cut();
    assert_eq!(installed(&rig, job).await, "slot-2");
    let counters = rig.installs.push_counters(job).expect("the push ran");
    assert!(
        counters.attempts >= 2,
        "the push began again on the new session: {counters:?}"
    );
    // Acknowledged bytes are not sent again: what was sent over all attempts is the file plus at
    // most one window (nine chunks of 32 KiB) of bytes the board had not acknowledged.
    assert!(
        counters.bytes_sent <= size + 9 * 32 * 1024,
        "bytes resent beyond the window: sent {} for a file of {size}",
        counters.bytes_sent
    );
    assert_eq!(rig.node.runs_started(names::SLOTS_INSTALL_FILE), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_lost_during_the_boards_install_never_starts_install_file_twice() {
    let rig = Rig::start("lost-install").await;
    rig.node.hold_installs(true);
    let path = write_bundle(
        &rig.dir.join("bundles"),
        "one.tar",
        &bundle_bytes("slot-2", "2.0.0", None),
    );
    let job = rig
        .installs
        .submit(device(), &rig.address(), BOARD, true, request(path, false))
        .await
        .expect("queued");
    until("the board to start the install", || {
        rig.node.runs_started(names::SLOTS_INSTALL_FILE) == 1
    })
    .await;
    rig.relay.cut();
    stage_is(&rig, job, "the install to wait for the board", |stage| {
        matches!(stage, Stage::WaitingForBoard { .. })
    })
    .await;
    rig.node.hold_installs(false);
    assert_eq!(installed(&rig, job).await, "slot-2");
    assert_eq!(
        rig.node.runs_started(names::SLOTS_INSTALL_FILE),
        1,
        "install_file was issued once"
    );
    assert!(list(&rig).await.get("slot-2").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_stops_the_install_on_the_board_and_leaves_no_slot() {
    let rig = Rig::start("cancel").await;
    rig.node.hold_installs(true);
    let path = write_bundle(
        &rig.dir.join("bundles"),
        "c.tar",
        &bundle_bytes("slot-2", "2.0.0", None),
    );
    let job = rig
        .installs
        .submit(device(), &rig.address(), BOARD, true, request(path, false))
        .await
        .expect("queued");
    until("the install to start", || {
        rig.node.runs_started(names::SLOTS_INSTALL_FILE) == 1
    })
    .await;
    assert!(rig.installs.cancel(job).await, "the job was active");
    assert_eq!(rig.installs.cancel_state(job), Some(CancelState::Confirmed));
    assert_eq!(rig.installs.stage(job), Some(Stage::Cancelled));
    rig.node.hold_installs(false);
    assert!(
        list(&rig).await.get("slot-2").is_none(),
        "nothing was installed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn set_default_changes_the_default_and_refuses_what_the_board_refuses() {
    let rig = Rig::start("default").await;
    install_bundle(&rig, "slot-2", "2.0.0").await;
    // slot-2 is untried, so the board refuses it as default: its words come back verbatim.
    match rig.slots.set_default("slot-2", false).await {
        Err(SlotError::Board(error)) => {
            assert!(error.to_string().contains("not marked good"), "{error}")
        }
        other => panic!("expected the board's refusal, got {other:?}"),
    }
    rig.slots
        .set_default("slot-1", false)
        .await
        .expect("slot-1 is good: it stays the default");
    assert_eq!(list(&rig).await.default, "slot-1");
    let unknown = rig.slots.set_default("slot-9", false).await;
    assert!(matches!(unknown, Err(SlotError::Board(_))), "{unknown:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trial_confirms_itself_and_the_resolved_record_says_so() {
    let rig = Rig::start("trial-confirm").await;
    install_bundle(&rig, "slot-2", "2.0.0").await;
    rig.slots
        .start_trial("slot-2", false, false, false)
        .await
        .expect("the trial starts");
    let pending = list(&rig).await;
    assert_eq!(
        pending.trial.as_ref().map(|trial| trial.slot.as_str()),
        Some("slot-2")
    );
    rig.node.slot_clock_advance(30);
    until("the board to confirm the trial", || {
        rig.node.model().slots.resolved.is_some()
    })
    .await;
    let done = list(&rig).await;
    assert!(done.trial.is_none(), "the pending trial is cleared");
    let resolved = done.resolved.expect("resolved is written");
    assert_eq!(resolved.slot, "slot-2");
    assert_eq!(resolved.outcome, Resolution::Confirmed);
    assert_eq!(resolved.by, "auto");
    assert_eq!(
        done.default, "slot-2",
        "a confirmed trial becomes the default"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trial_that_is_not_healthy_falls_back_at_the_deadline_with_the_reason() {
    let rig = Rig::start("trial-fallback").await;
    install_bundle(&rig, "slot-2", "2.0.0").await;
    rig.node.slot_set_healthy(false);
    rig.slots
        .start_trial("slot-2", false, false, false)
        .await
        .expect("the trial starts");
    rig.node.slot_clock_advance(600);
    until("the fall back", || {
        rig.node.model().slots.resolved.is_some()
    })
    .await;
    let done = list(&rig).await;
    let resolved = done.resolved.expect("resolved is written");
    assert_eq!(resolved.outcome, Resolution::FellBack);
    assert!(!resolved.reason.is_empty(), "the board's reason is kept");
    assert_eq!(done.running, "slot-1", "the default comes back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_boots_that_never_reach_the_daemon_fall_back_and_name_the_count() {
    let rig = Rig::start("trial-boots").await;
    install_bundle(&rig, "slot-2", "2.0.0").await;
    rig.node.slot_boots_without_daemon(3);
    rig.slots
        .start_trial("slot-2", false, false, false)
        .await
        .expect("the trial starts");
    let actions = BoardActions::new(rig.handle.clone());
    for _ in 0..3 {
        // Each restart is a boot that never reaches the daemon.
        let _ = actions.run(&reboot_action(), &[]).await;
    }
    until("the fall back after three boots", || {
        rig.node.model().slots.resolved.is_some()
    })
    .await;
    let done = list(&rig).await;
    let resolved = done.resolved.expect("resolved is written");
    assert_eq!(resolved.outcome, Resolution::FellBack);
    assert!(
        resolved.reason.contains('3'),
        "the reason names the count: {}",
        resolved.reason
    );
    assert!(
        done.trial.is_none(),
        "the falling-back trial is no longer pending"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_removal_guard_refuses_running_starred_and_the_board_keeps_the_last_word() {
    let rig = Rig::start("remove").await;
    install_bundle(&rig, "slot-2", "2.0.0").await;
    // Running and default: refused here, before the board is asked.
    let running = rig.slots.remove("slot-1", false).await;
    assert!(
        matches!(running, Err(SlotError::Removal(RemoveRefusal::Running(_)))),
        "{running:?}"
    );
    // Starred: refused here unless the person says so.
    rig.slots
        .set_starred("slot-2", true)
        .await
        .expect("slot-2 is starred");
    let starred = rig.slots.remove("slot-2", false).await;
    assert!(
        matches!(starred, Err(SlotError::Removal(RemoveRefusal::Starred(_)))),
        "{starred:?}"
    );
    // Said so explicitly: the board still refuses a pinned slot, and its words come back.
    let board_says = rig.slots.remove("slot-2", true).await;
    match board_says {
        Err(SlotError::Board(error)) => assert!(error.to_string().contains("pinned"), "{error}"),
        other => panic!("expected the board's refusal, got {other:?}"),
    }
    rig.slots
        .set_starred("slot-2", false)
        .await
        .expect("unstarred");
    rig.slots
        .remove("slot-2", false)
        .await
        .expect("an unstarred, unused slot is removed");
    assert!(list(&rig).await.get("slot-2").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_board_without_the_slot_contract_has_no_slots_and_is_asked_nothing() {
    let rig = Rig::start("no-slots").await;
    let plain = BoardSlots::new(rig.handle.clone(), BOARD, false);
    assert!(matches!(plain.list().await, Err(SlotError::NoSlots)));
    assert_eq!(
        rig.node.runs_started(names::SLOTS_LIST),
        0,
        "the board was not asked"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_list_over_the_size_bound_is_passed_on_by_name_and_not_truncated() {
    let rig = Rig::start("oversize").await;
    {
        let mut model = rig.node.model();
        let template = model.slots.entries[0].clone();
        for number in 0..300_u64 {
            let mut entry = template.clone();
            entry.id = format!("bulk-{number}");
            entry.seq = 100 + number;
            model.slots.entries.push(entry);
        }
    }
    match rig.slots.list().await {
        Err(SlotError::Board(error)) => {
            let text = error.to_string();
            assert!(
                text.contains("60 KiB"),
                "the board's words are passed on: {text}"
            );
        }
        other => panic!("expected the size refusal, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_signature_outcome_of_an_installed_slot_is_not_checked_when_unsigned() {
    let rig = Rig::start("signature").await;
    install_bundle(&rig, "slot-2", "2.0.0").await;
    let slot = list(&rig).await.get("slot-2").expect("listed").clone();
    assert_eq!(
        slot.signature,
        atlas_slots::Reported::Known(BoardOutcome::NotCheckedUnsigned)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_install_interrupted_by_a_replaced_board_ends_and_sends_nothing_to_the_other() {
    let rig = Rig::start("replaced").await;
    rig.node.hold_installs(true);
    let path = write_bundle(
        &rig.dir.join("bundles"),
        "r.tar",
        &bundle_bytes("slot-2", "2.0.0", None),
    );
    let job = rig
        .installs
        .submit(device(), &rig.address(), BOARD, true, request(path, false))
        .await
        .expect("queued");
    until("the first board to start the install", || {
        rig.node.runs_started(names::SLOTS_INSTALL_FILE) == 1
    })
    .await;
    // The address now answers with another board: the first node goes away, and a different
    // node (a new key, a new keys directory) answers on the same port.
    let port = rig.node.network().port();
    let dir = rig.dir.clone();
    let Rig {
        node,
        relay,
        installs,
        ..
    } = rig;
    drop(node);
    let replacement =
        atlas_sim_node::SimNode::start_with(&dir.join("replacement"), &common::kind(), Some(port))
            .expect("the replacement node starts on the same port");
    // Keep the relay alive for the whole test.
    let _relay = relay;
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        while !installs.stage(job).is_some_and(|stage| stage.is_final()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "the install did not end: {:?}",
        installs.stage(job)
    );
    match installs.stage(job) {
        Some(Stage::Failed { reason }) => assert_eq!(reason, atlas_board_slots::install::REPLACED),
        other => panic!("expected the replaced-board ending, got {other:?}"),
    }
    assert_eq!(
        replacement.runs_started(names::SLOTS_ACCEPT),
        0,
        "nothing was sent to the other board"
    );
    assert_eq!(
        replacement.runs_started(names::SLOTS_INSTALL_FILE),
        0,
        "nothing was installed on it"
    );
}
