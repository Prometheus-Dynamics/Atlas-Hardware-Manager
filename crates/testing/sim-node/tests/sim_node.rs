//! The simulated node, proven against Orion's own client and comm core.
#![cfg(target_os = "linux")]

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use atlas_sim_node::bundle;
use atlas_sim_node::names::{self, SURFACE};
use atlas_sim_node::slots::code;
use atlas_sim_node::slots::parse_keys;
use atlas_sim_node::{Behaviour, DEADLINE, Hello, Kind, LinkFault, Mode, NODE_NAME, SimNode, call};
use ed25519_dalek::SigningKey;
use orion::actions::{ActionCall, Finished, Outcome, Update};
use orion::topics::FieldId;
use orion::{Address, Orion};
use serde_json::Value;

/// How often a wait looks again at the condition it waits for. Only [`DEADLINE`] ends a wait
/// that never comes true; the poll interval is never a claim about how long anything takes.
const POLL: Duration = Duration::from_millis(10);

/// Asks `check` until it has an answer. Load can make any step slow, so nothing here assumes a
/// step has happened by some time: the wait ends when the answer is there, or at [`DEADLINE`].
fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(POLL);
    }
}

/// Waits until the slots report `state` (the listing's `state` word).
fn wait_for_state(orion: &Orion, state: &str) {
    wait_for(&format!("the slots reaching {state}"), || {
        (list(orion)["state"] == state).then_some(())
    });
}

/// A node for one test. Each test has its own name, so its own directory: the node's socket and
/// state never meet another test's.
fn start(name: &str) -> SimNode {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sim-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    SimNode::start(&dir).expect("the simulated node starts")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A bundle for slot `id`, signed by `key` when given.
fn bundle_for(id: &str, version: &str, key: Option<&SigningKey>) -> Vec<u8> {
    let rootfs = format!("rootfs of {id} at {version}\n")
        .repeat(900)
        .into_bytes();
    let boot: [(&str, &[u8]); 2] = [
        ("cmdline.txt", b"console=ttyS0 root=/dev/dm-0\n"),
        ("kernel.img", b"kernel bytes"),
    ];
    let manifest = bundle::manifest(id, version, &rootfs, &boot);
    let signature = key.map(|key| bundle::sign(&manifest, key));
    bundle::build(&manifest, signature.as_ref(), &rootfs, &boot)
}

/// The name a test pushes its bundle under.
const BUNDLE: &str = "bundle.tar";

/// Pushes `bytes` as a file into the incoming directory, as a sender does over Orion.
fn push(orion: &Orion, name: &str, bytes: &[u8]) {
    let destination = format!("{}{name}", names::INCOMING);
    orion
        .files()
        .push(&destination, &mut Cursor::new(bytes), |_, _| {})
        .expect("the bundle is pushed");
}

/// Starts the install job for a pushed bundle.
fn start_install(orion: &Orion, name: &str, pin: bool, keep_old: bool) -> orion::actions::Run {
    let args = format!("{name} {pin} {keep_old}");
    orion
        .run(&ActionCall {
            name: names::SLOTS_INSTALL_FILE,
            target: atlas_sim_node::node::target(),
            args: args.as_bytes(),
            deadline_ms: 0,
            detached: false,
        })
        .expect("the install starts")
}

/// Accepts, pushes and installs a bundle, and says how the install ended.
fn install(orion: &Orion, bundle: &[u8], pin: bool, keep_old: bool) -> Finished {
    let accepted = call(
        orion,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    )
    .expect("accept answers");
    if accepted.outcome != Outcome::Succeeded {
        return accepted;
    }
    push(orion, BUNDLE, bundle);
    start_install(orion, BUNDLE, pin, keep_old)
        .finish(DEADLINE)
        .expect("the install ends")
}

/// A bundle of about four mebibytes. Only the large-transfer test uses it: every other test
/// installs a small bundle and holds the install where it is, so nothing depends on how fast
/// the machine moves bytes.
fn big_bundle(id: &str) -> Vec<u8> {
    let rootfs = format!("rootfs of {id}\n").into_bytes();
    let blob = vec![7_u8; 4 * 1024 * 1024];
    let boot = [
        ("cmdline.txt", &b"console=ttyS0\n"[..]),
        ("blob.bin", &blob[..]),
    ];
    let manifest = bundle::manifest(id, "2.0.0", &rootfs, &boot);
    bundle::build(&manifest, None, &rootfs, &boot)
}

fn list(orion: &Orion) -> Value {
    let finished = call(orion, names::SLOTS_LIST, b"").expect("list answers");
    assert_eq!(finished.outcome, Outcome::Succeeded);
    serde_json::from_slice(&finished.output).expect("list is JSON")
}

fn slot<'a>(listing: &'a Value, id: &str) -> &'a Value {
    listing["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|slot| slot["id"] == id)
        .unwrap_or_else(|| panic!("no slot {id} in {listing}"))
}

fn ok(orion: &Orion, name: &str, args: &[u8]) -> Finished {
    let finished = call(orion, name, args).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(
        finished.outcome,
        Outcome::Succeeded,
        "{name} {args:?}: {}",
        finished.detail
    );
    finished
}

fn refused(orion: &Orion, name: &str, args: &[u8]) -> Finished {
    let finished = call(orion, name, args).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_ne!(
        finished.outcome,
        Outcome::Succeeded,
        "{name} {args:?} should be refused"
    );
    finished
}

#[test]
fn every_surface_method_answers_and_its_result_is_right() {
    let node = start("surface");
    let orion = node.connect("surface").expect("a session");
    let mut exercised: Vec<&str> = Vec::new();

    // list: the board's one slot, running and default, good.
    let listing = list(&orion);
    assert_eq!(listing["slot_active"], "slot-1");
    assert_eq!(slot(&listing, "slot-1")["good"], "yes");
    exercised.push(names::SLOTS_LIST);

    // accept answers when there is room, and changes nothing else.
    ok(&orion, names::SLOTS_ACCEPT, b"26000");
    exercised.push(names::SLOTS_ACCEPT);

    // install_file for a name nothing was pushed under is refused and changes nothing.
    let missing = refused(
        &orion,
        names::SLOTS_INSTALL_FILE,
        b"nothing.tar false false",
    );
    assert_eq!(missing.outcome, Outcome::Rejected);
    exercised.push(names::SLOTS_INSTALL_FILE);

    // install: a long job that ends with the new slot id. Installed slots start untried.
    let two = bundle_for("slot-2", "2.0.0", None);
    let done = install(&orion, &two, false, false);
    assert_eq!(done.outcome, Outcome::Succeeded, "{}", done.detail);
    assert_eq!(text(&done.output), "slot-2");
    assert_eq!(slot(&list(&orion), "slot-2")["good"], "untried");

    // trial restarts into slot-2; confirm makes it the default with slot-1 as previous.
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    assert_eq!(list(&orion)["slot_active"], "slot-2");
    exercised.push(names::SLOTS_TRIAL);
    ok(&orion, names::SLOTS_CONFIRM, b"");
    let confirmed = list(&orion);
    assert_eq!(confirmed["slot_default"], "slot-2");
    assert_eq!(confirmed["slot_previous"], "slot-1");
    exercised.push(names::SLOTS_CONFIRM);

    // set_default needs a good slot: slot-1 is, so it can be made the default again.
    ok(&orion, names::SLOTS_SET_DEFAULT, b"slot-1 false");
    assert_eq!(list(&orion)["slot_default"], "slot-1");
    refused(&orion, names::SLOTS_SET_DEFAULT, b"slot-9 false");
    exercised.push(names::SLOTS_SET_DEFAULT);

    // pin: a pinned slot will not be removed.
    ok(&orion, names::SLOTS_PIN, b"slot-2 true");
    assert_eq!(slot(&list(&orion), "slot-2")["pinned"], true);
    let pinned = refused(&orion, names::SLOTS_REMOVE, b"slot-2");
    assert_eq!(pinned.outcome, Outcome::Rejected);
    exercised.push(names::SLOTS_PIN);

    // remove: the default slot is busy; an unknown id is not found.
    refused(&orion, names::SLOTS_REMOVE, b"slot-1");
    refused(&orion, names::SLOTS_REMOVE, b"..");
    ok(&orion, names::SLOTS_PIN, b"slot-2 false");
    // slot-2 is still the running slot until the next boot, so it is busy until then.
    refused(&orion, names::SLOTS_REMOVE, b"slot-2");
    ok(&orion, names::REBOOT, b"");
    ok(&orion, names::SLOTS_REMOVE, b"slot-2");
    assert!(slot_missing(&list(&orion), "slot-2"));
    exercised.push(names::SLOTS_REMOVE);

    // set_keep: zero keeps all; any number runs housekeeping.
    ok(&orion, names::SLOTS_SET_KEEP, b"0");
    ok(&orion, names::SLOTS_SET_KEEP, b"3");
    exercised.push(names::SLOTS_SET_KEEP);

    // reboot restarts into the default.
    ok(&orion, names::REBOOT, b"");
    assert_eq!(list(&orion)["slot_active"], "slot-1");
    exercised.push(names::REBOOT);

    let mut surface: Vec<&str> = SURFACE.iter().map(|method| method.action).collect();
    surface.sort_unstable();
    exercised.sort_unstable();
    assert_eq!(
        exercised, surface,
        "every method of the I5 surface is exercised"
    );
}

fn slot_missing(listing: &Value, id: &str) -> bool {
    !listing["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .any(|slot| slot["id"] == id)
}

#[test]
fn the_board_answers_its_own_actions_and_data_partition() {
    let node = start("own");
    let orion = node.connect("own").expect("a session");
    assert_eq!(ok(&orion, names::LOCATE, b"").output, b"blinking");
    assert_eq!(ok(&orion, names::SELF_TEST, b"").output, b"passed");
    assert_eq!(ok(&orion, names::SET_CLOCK, b"").output, b"clock set");
    assert!(node.model().clock_offset_s.is_none());
    assert!(text(&ok(&orion, names::STATUS, b"").output).contains("running=slot-1"));
    assert!(text(&ok(&orion, names::SUPPORT_BUNDLE, b"").output).contains("slot-1"));

    let backup = ok(&orion, names::DATA_BACKUP, b"").output;
    ok(&orion, names::FS_WRITE, b"notes.txt 68656c6c6f");
    assert_eq!(
        ok(&orion, names::FS_READ, b"notes.txt").output,
        b"68656c6c6f"
    );
    assert!(text(&ok(&orion, names::FS_LIST, b"").output).contains("notes.txt"));
    ok(&orion, names::FS_REMOVE, b"notes.txt");
    refused(&orion, names::FS_READ, b"notes.txt");
    ok(&orion, names::DATA_RESTORE, &backup);
    assert!(node.model().data.contains_key("config/board.toml"));
    refused(&orion, names::DATA_RESTORE, b"zz");
    assert!(
        node.model().data.contains_key("config/board.toml"),
        "a bad restore changes nothing"
    );
}

#[test]
fn a_refused_run_changes_nothing() {
    let node = start("refuse");
    let orion = node.connect("refuse").expect("a session");
    node.script().push_action(
        names::SLOTS_REMOVE,
        Behaviour::Refuse("held by the test".into()),
    );
    let finished = call(&orion, names::SLOTS_REMOVE, b"slot-1").expect("an outcome");
    assert_eq!(finished.outcome, Outcome::Rejected);
    assert_eq!(finished.detail, "held by the test");
    assert_eq!(list(&orion)["slot_default"], "slot-1");
}

#[test]
fn a_failed_run_changes_nothing() {
    let node = start("fail");
    let orion = node.connect("fail").expect("a session");
    node.script().push_action(
        names::SLOTS_CONFIRM,
        Behaviour::Fail("the board did not answer".into()),
    );
    let finished = call(&orion, names::SLOTS_CONFIRM, b"").expect("an outcome");
    assert_eq!(finished.outcome, Outcome::Failed);
    assert_eq!(list(&orion)["slot_default"], "slot-1");
}

#[test]
fn a_stalled_self_test_is_cancelled_with_no_change() {
    let node = start("stall");
    let orion = node.connect("stall").expect("a session");
    node.script()
        .push_action(names::SELF_TEST, Behaviour::Stall);
    let mut run = orion
        .run(&ActionCall {
            name: names::SELF_TEST,
            target: atlas_sim_node::node::target(),
            args: b"",
            deadline_ms: 0,
            detached: false,
        })
        .expect("the run starts");
    wait_for("the self-test to stall", || {
        (node.stalled_runs() == 1).then_some(())
    });
    run.cancel().expect("cancel is sent");
    assert_eq!(
        run.finish(DEADLINE).expect("it ends").outcome,
        Outcome::Cancelled
    );
    assert!(node.model().slots.trial.is_none());
}

/// Lemnos I6c, the meaning of a plain cancel: it ends its own run and nothing else. The caller's
/// session is not taken as gone, so it still carries calls, and another session's run is not
/// touched. (The node cannot show a driver's per-caller hold here; that is Lemnos's, and Orion's
/// reason 2 is not sent yet. See docs/prs/B23.md.)
#[test]
fn a_plain_cancel_ends_its_own_run_and_the_session_carries_on() {
    let node = start("cancel-own");
    let cancelling = node.connect("cancelling").expect("a session");
    let other = node.connect("other").expect("a session");
    node.script()
        .push_action(names::SELF_TEST, Behaviour::Stall);
    let mut run = cancelling
        .run(&ActionCall {
            name: names::SELF_TEST,
            target: atlas_sim_node::node::target(),
            args: b"",
            deadline_ms: 0,
            detached: false,
        })
        .expect("the run starts");
    wait_for("the self-test to stall", || {
        (node.stalled_runs() == 1).then_some(())
    });
    run.cancel().expect("cancel is sent");
    assert_eq!(
        run.finish(DEADLINE).expect("it ends").outcome,
        Outcome::Cancelled,
        "the plain cancel ends its own run as cancelled"
    );

    // The cancelling session is still there: it carries calls after the cancel.
    assert_eq!(list(&cancelling)["state"], list(&other)["state"]);
    // Another session's run is untouched: it runs to its end.
    let finished = ok(&other, names::SELF_TEST, b"");
    assert_eq!(finished.outcome, Outcome::Succeeded);
}

#[test]
fn cancel_discards_the_pushed_file_and_the_install() {
    let node = start("cancel");
    let orion = node.connect("cancel").expect("a session");
    node.hold_installs(true);
    let bundle = bundle_for("slot-2", "2.0.0", None);
    ok(
        &orion,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    );
    push(&orion, BUNDLE, &bundle);
    let mut run = start_install(&orion, BUNDLE, false, false);
    wait_for_state(&orion, "staging");
    run.cancel().expect("cancel is sent");
    assert_eq!(
        run.finish(DEADLINE).expect("it ends").outcome,
        Outcome::Cancelled
    );
    let listing = list(&orion);
    assert_eq!(listing["state"], "cancelled");
    assert!(
        slot_missing(&listing, "slot-2"),
        "nothing was installed as a slot"
    );
    assert!(
        !node
            .shelf()
            .committed
            .lock()
            .expect("the shelf")
            .contains_key(&format!("{}{BUNDLE}", names::INCOMING)),
        "the pushed file is removed when the job ends"
    );
}

#[test]
fn a_starter_that_disconnects_has_its_install_cancelled() {
    let node = start("disconnect");
    let starter = node.connect("starter").expect("a session");
    node.hold_installs(true);
    let bundle = bundle_for("slot-2", "2.0.0", None);
    ok(
        &starter,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    );
    push(&starter, BUNDLE, &bundle);
    let run = start_install(&starter, BUNDLE, false, false);
    wait_for_state(&starter, "staging");
    drop(run);
    drop(starter);

    let other = node.connect("after").expect("a session");
    wait_for_state(&other, "cancelled");
}

#[test]
fn a_detached_install_survives_its_starter_and_can_be_attached_to_again() {
    let node = start("detached");
    let starter = node.connect("starter").expect("a session");
    node.hold_installs(true);
    let bundle = bundle_for("slot-2", "2.0.0", None);
    ok(
        &starter,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    );
    push(&starter, BUNDLE, &bundle);
    let args = format!("{BUNDLE} false false");
    let run = starter
        .run(&ActionCall {
            name: names::SLOTS_INSTALL_FILE,
            target: atlas_sim_node::node::target(),
            args: args.as_bytes(),
            deadline_ms: 0,
            detached: true,
        })
        .expect("the detached install starts");
    let call_number = run.call();
    drop(run);
    drop(starter);

    // The starter is gone, and the install is still held in its place. The attach can reach the
    // router before the starter's session close has been handled, so an attach that is not
    // answered yet is asked again until the run is found.
    let other = node.connect("later").expect("a session");
    let mut attached = wait_for("the detached run to be found", || {
        let mut attached = other.attach(call_number).ok()?;
        let answered = attached.wait(POLL).is_ok();
        answered.then_some(attached)
    });
    // Now let the install land: the run that was attached to is the one that finishes.
    node.hold_installs(false);
    let finished = attached.finish(DEADLINE).expect("the run ends");
    assert_eq!(finished.outcome, Outcome::Succeeded, "{}", finished.detail);
    assert_eq!(text(&finished.output), "slot-2");
    assert_eq!(list(&other)["state"], "idle");
}

#[test]
fn a_plain_run_cannot_be_attached_to_by_another_session() {
    let node = start("starter-only");
    let starter = node.connect("starter").expect("a session");
    node.hold_installs(true);
    let bundle = bundle_for("slot-2", "2.0.0", None);
    ok(
        &starter,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    );
    push(&starter, BUNDLE, &bundle);
    let run = start_install(&starter, BUNDLE, false, false);
    wait_for_state(&starter, "staging");
    let call_number = run.call();

    // Another session cannot attach to a run that was not started detached. The install is held,
    // so the only way the intruder's attach gets an answer is a refusal: a run it was handed
    // could not finish while held.
    let other = node.connect("intruder").expect("a session");
    let mut attached = other.attach(call_number).expect("the attach is sent");
    let refused = wait_for("the intruder's attach to be refused", || {
        match attached.wait(POLL) {
            Err(_) => Some(true),
            Ok(Some(Update::Finished(finished))) => Some(finished.outcome != Outcome::Succeeded),
            Ok(_) => None,
        }
    });
    assert!(refused, "a plain run is not handed to another session");
    drop(run);
    drop(starter);
    drop(other);
}

#[test]
fn a_service_restart_loses_the_running_install_and_a_new_one_starts_clean() {
    let node = start("restart");
    let orion = node.connect("restart").expect("a session");
    node.hold_installs(true);
    let bundle = bundle_for("slot-2", "2.0.0", None);
    ok(
        &orion,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    );
    push(&orion, BUNDLE, &bundle);
    let mut run = start_install(&orion, BUNDLE, false, false);
    // Restart only once the install has started: the request and the restart are both handled
    // by the node, in its own order.
    wait_for_state(&orion, "staging");

    node.restart_daemon();
    let lost = run.finish(DEADLINE).expect("it ends");
    assert_eq!(lost.outcome, Outcome::Failed, "{}", lost.detail);
    assert!(lost.detail.starts_with("-4"), "{}", lost.detail);
    let listing = list(&orion);
    assert_eq!(listing["state"], "error");
    assert_eq!(
        listing["error"],
        "an install was interrupted: the service restarted"
    );

    // Nothing resumes: a fresh install of the same bundle starts from zero and lands.
    node.hold_installs(false);
    let two = bundle_for("slot-2", "2.0.0", None);
    let again = install(&orion, &two, false, false);
    assert_eq!(again.outcome, Outcome::Succeeded, "{}", again.detail);
    assert_eq!(list(&orion)["state"], "idle");
}

#[test]
fn a_scripted_restart_loses_jobs_at_its_step() {
    let node = start("scripted-restart");
    let orion = node.connect("scripted").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    node.script()
        .push_action(names::STATUS, Behaviour::RestartDaemon);
    let finished = call(&orion, names::STATUS, b"").expect("an outcome");
    assert_eq!(finished.outcome, Outcome::Failed);
    let landed = install(&orion, &two, false, false);
    assert_eq!(landed.outcome, Outcome::Succeeded, "{}", landed.detail);
}

#[test]
fn a_trial_that_does_not_confirm_falls_back_and_marks_the_slot_failed() {
    let node = start("trial");
    let orion = node.connect("trial").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );

    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    assert_eq!(list(&orion)["slot_active"], "slot-2");
    ok(&orion, names::REBOOT, b"");
    let fell_back = list(&orion);
    assert_eq!(
        fell_back["slot_active"], "slot-1",
        "fell back to the default"
    );
    assert_eq!(fell_back["state"], "rolled-back");
    assert_eq!(slot(&fell_back, "slot-2")["failed"], true);
    assert_eq!(slot(&fell_back, "slot-2")["good"], "no");

    // A failed slot is not made the default unless forced.
    refused(&orion, names::SLOTS_SET_DEFAULT, b"slot-2 false");
    refused(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false true");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    assert_eq!(list(&orion)["slot_default"], "slot-2");
}

#[test]
fn a_trial_with_keep_default_leaves_the_default_alone_on_confirm() {
    let node = start("keep-default");
    let orion = node.connect("keep-default").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 true false false");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    let listing = list(&orion);
    assert_eq!(listing["slot_default"], "slot-1");
    assert_eq!(listing["slot_active"], "slot-2");
    assert_eq!(slot(&listing, "slot-2")["good"], "yes");
}

#[test]
fn a_trial_confirmed_becomes_the_default_and_survives_a_reboot() {
    let node = start("confirm");
    let orion = node.connect("confirm").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    ok(&orion, names::REBOOT, b"");
    let listing = list(&orion);
    assert_eq!(listing["slot_default"], "slot-2");
    assert_eq!(listing["slot_active"], "slot-2");
    assert_eq!(slot(&listing, "slot-2")["failed"], false);
}

#[test]
fn a_confirm_changes_the_default_slot_and_not_the_running_slot_fact() {
    let node = start("confirm-running");
    let orion = node.connect("confirm-running").expect("a session");
    assert_eq!(hello_fact(&orion, names::HELLO_VERSION_SLOT), "slot-1");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    assert_eq!(list(&orion)["slot_default"], "slot-2");
    // The running slot is written at boot. A confirm does not change it.
    assert_eq!(hello_fact(&orion, names::HELLO_VERSION_SLOT), "slot-1");
    assert_eq!(hello_fact(&orion, names::HELLO_VERSION_OS), "1.0.0");
}

#[test]
fn housekeeping_keeps_the_last_n_and_the_pinned_ones_and_keep_old_skips_it() {
    let node = start("housekeeping");
    let orion = node.connect("housekeeping").expect("a session");
    ok(&orion, names::SLOTS_SET_KEEP, b"2");
    let pinned = bundle_for("pinned", "1.0.0", None);
    assert_eq!(
        install(&orion, &pinned, true, false).outcome,
        Outcome::Succeeded
    );
    for version in 2..=5 {
        let id = format!("slot-{}", version + 1);
        let landed = install(
            &orion,
            &bundle_for(&id, &format!("{version}.0.0"), None),
            false,
            false,
        );
        assert_eq!(landed.outcome, Outcome::Succeeded, "{}", landed.detail);
    }
    let kept: Vec<String> = list(&orion)["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .map(|slot| slot["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        kept.contains(&"pinned".to_string()),
        "a pinned slot stays: {kept:?}"
    );
    assert!(
        kept.contains(&"slot-1".to_string()),
        "the running and default slot stays: {kept:?}"
    );
    assert!(kept.len() <= 3, "keep-last and the pin: {kept:?}");

    // keep_old on an install skips housekeeping for it.
    let before = kept.len();
    assert_eq!(
        install(&orion, &bundle_for("kept-old", "9.0.0", None), false, true).outcome,
        Outcome::Succeeded
    );
    assert_eq!(
        list(&orion)["slots"].as_array().expect("slots").len(),
        before + 1
    );
}

#[test]
fn a_bundle_is_checked_and_a_bad_one_leaves_nothing() {
    let node = start("bundles");
    let orion = node.connect("bundles").expect("a session");
    let good = bundle_for("slot-2", "2.0.0", None);

    // A tar with a symlink member.
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    builder
        .append_link(&mut header, "rootfs.erofs", "/etc/passwd")
        .expect("a link");
    let link = builder.into_inner().expect("a tar");
    let cases: Vec<(&str, Vec<u8>, i32)> = vec![
        ("a symlink", link, code::INVALID_ARGUMENT),
        (
            "a broken tar",
            b"not a tar".to_vec(),
            code::INVALID_ARGUMENT,
        ),
        (
            "an unknown member",
            with_extra_member(&good),
            code::INVALID_ARGUMENT,
        ),
        (
            "a running id",
            bundle_for("slot-1", "2.0.0", None),
            code::BUSY,
        ),
    ];
    for (what, bytes, expected) in cases {
        let finished = install(&orion, &bytes, false, false);
        assert_ne!(
            finished.outcome,
            Outcome::Succeeded,
            "{what} must be refused"
        );
        assert!(
            finished.detail.starts_with(&format!("{expected}:")),
            "{what}: {}",
            finished.detail
        );
    }
    let listing = list(&orion);
    assert_eq!(
        listing["slots"].as_array().expect("slots").len(),
        1,
        "nothing was left behind"
    );
}

/// The good bundle with one more member the manifest does not list.
fn with_extra_member(good: &[u8]) -> Vec<u8> {
    let mut archive = tar::Archive::new(Cursor::new(good));
    let mut builder = tar::Builder::new(Vec::new());
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        let path = entry.path().expect("path").into_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).expect("bytes");
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, bytes.as_slice())
            .expect("member");
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(3);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "boot/extra.txt", &b"abc"[..])
        .expect("extra");
    builder.into_inner().expect("tar")
}

#[test]
fn security_off_installs_everything_and_still_shows_each_signature() {
    let node = start("security-off");
    let orion = node.connect("security-off").expect("a session");
    let key = SigningKey::from_bytes(&[7; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_sim_node::slots::TrustedKey {
            public: key.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let signed = bundle_for("signed", "2.0.0", Some(&key));
    assert_eq!(
        install(&orion, &signed, false, true).outcome,
        Outcome::Succeeded
    );
    let unsigned = bundle_for("unsigned", "3.0.0", None);
    assert_eq!(
        install(&orion, &unsigned, false, true).outcome,
        Outcome::Succeeded
    );
    let stranger = SigningKey::from_bytes(&[9; 32]);
    let untrusted = bundle_for("untrusted", "4.0.0", Some(&stranger));
    assert_eq!(
        install(&orion, &untrusted, false, true).outcome,
        Outcome::Succeeded
    );
    let listing = list(&orion);
    assert_eq!(slot(&listing, "signed")["signed"], "trusted");
    assert_eq!(slot(&listing, "signed")["signer"], "Test Owner");
    assert_eq!(slot(&listing, "unsigned")["signed"], "unsigned");
    assert_eq!(slot(&listing, "unsigned")["signer"], "");
    assert_eq!(slot(&listing, "untrusted")["signed"], "untrusted");
    assert_eq!(slot(&listing, "untrusted")["signer"], "");
    assert!(
        !listing.to_string().contains("unverified"),
        "the old label is gone: {listing}"
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn a_key_file_is_hex_key_and_owner_per_line_and_a_line_without_an_owner_refuses_it() {
    let public = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
    let text = format!(
        "# the release keys\n\n{} Test Owner\n{}\tSecond Owner  \n",
        hex(&public),
        hex(&public)
    );
    let keys = parse_keys(&text).expect("a good key file");
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].public, public);
    assert_eq!(keys[0].owner, "Test Owner");
    assert_eq!(keys[1].owner, "Second Owner");

    let no_owner = format!("# comment\n{}\n", hex(&public));
    assert!(parse_keys(&no_owner).is_err(), "a key with no owner");
    assert!(parse_keys(&format!("{} owner\n", "ab".repeat(31))).is_err());
    assert!(parse_keys(&format!("{} owner\n", "zz".repeat(32))).is_err());

    // A refused file loads as no keys at all.
    let mut slots = atlas_sim_node::slots::Slots::new();
    assert_eq!(slots.load_keys(&text), Ok(2));
    assert!(slots.load_keys(&no_owner).is_err());
    assert!(slots.trusted.is_empty());
}

#[test]
fn a_layered_manifest_is_refused() {
    let node = start("layered");
    let orion = node.connect("layered").expect("a session");
    let rootfs = b"rootfs bytes".to_vec();
    let mut manifest = bundle::manifest("layered-1", "2.0.0", &rootfs, &[("cmdline.txt", b"x")]);
    manifest["layers"] = serde_json::json!([{ "id": "base" }]);
    let bytes = bundle::build(&manifest, None, &rootfs, &[("cmdline.txt", b"x")]);
    let finished = install(&orion, &bytes, false, false);
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
    assert!(finished.detail.contains("layered"), "{}", finished.detail);
    assert_eq!(list(&orion)["slots"].as_array().expect("slots").len(), 1);
}

/// The text of one hello fact, as the node's own hello document carries it.
fn hello_fact(orion: &Orion, topic: &str) -> String {
    let fact = topic.strip_prefix("hello.").expect("a hello topic");
    wait_for(&format!("the hello fact {fact}"), || {
        let document = orion.node().expect("the node document");
        document
            .facts
            .iter()
            .find(|found| found.name == fact)
            .map(|found| text(&found.value))
    })
}

#[test]
fn a_node_publishes_the_hello_facts_a_board_publishes_and_a_restart_on_its_port_finds_it_again() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sim-hello-restart");
    let _ = std::fs::remove_dir_all(&dir);
    let recovery = Hello {
        mode: Mode::Recovery,
        ..Hello::default()
    };
    let first =
        SimNode::start_with(&dir, &Kind::WithLemnos(recovery), None).expect("the node starts");
    let port = first.network().port();
    {
        let orion = first.connect("reader").expect("a session");
        assert_eq!(hello_fact(&orion, names::HELLO_FAMILY), "simulated");
        assert_eq!(hello_fact(&orion, names::HELLO_SERIAL), "0123456789abcdef");
        assert_eq!(hello_fact(&orion, names::HELLO_STATE), "recovery");
        assert_eq!(hello_fact(&orion, names::HELLO_MODEL), "simulated-board");
        assert_eq!(hello_fact(&orion, names::HELLO_VERSION_SLOT), "slot-1");
        assert_eq!(
            hello_fact(&orion, names::HELLO_VERSION_PRODUCT),
            "simulated-product"
        );
        assert_eq!(hello_fact(&orion, names::HELLO_VERSION_OS), "1.0.0");
        assert_eq!(hello_fact(&orion, names::HELLO_VERSION_LEMNOS), "1.0.0");
        assert_eq!(
            hello_fact(&orion, names::HELLO_OFFERS),
            "system.identity/1 system.reboot/1 system.slots/1"
        );
    }

    // Stopped: the port is free, and the node is gone from it.
    drop(first);
    let stopped = Orion::connect(
        &Address::Network {
            host: format!("127.0.0.1:{port}"),
            key: None,
        },
        "after-stop",
    );
    assert!(stopped.is_err(), "a stopped node answers nothing");

    // Found again on the same port, running this time, with its name.
    let second = SimNode::start_with(&dir, &Kind::default(), Some(port)).expect("restart");
    assert_eq!(second.network().port(), port);
    let orion = second
        .connect_through_link("back")
        .expect("a session through the link");
    assert_eq!(hello_fact(&orion, names::HELLO_STATE), "running");
    // The name is the hello document's, not a fact.
    assert_eq!(orion.node().expect("the document").name, NODE_NAME);
    let answered = call(&orion, names::STATUS, b"").expect("status answers");
    assert_eq!(answered.outcome, Outcome::Succeeded);
}

#[test]
fn security_on_refuses_unsigned_and_untrusted_bundles_and_keeps_nothing() {
    let node = start("security-on");
    let orion = node.connect("security-on").expect("a session");
    node.model().slots.require_signed = true;

    // No trusted keys at all: the install does not start.
    let refusal = install(&orion, &bundle_for("slot-2", "2.0.0", None), false, false);
    assert_eq!(refusal.outcome, Outcome::Rejected);
    assert!(refusal.detail.starts_with("-1:"), "{}", refusal.detail);

    let trusted = SigningKey::from_bytes(&[7; 32]);
    let stranger = SigningKey::from_bytes(&[9; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_sim_node::slots::TrustedKey {
            public: trusted.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let unsigned = install(&orion, &bundle_for("unsigned", "2.0.0", None), false, false);
    assert!(unsigned.detail.starts_with("-1:"), "{}", unsigned.detail);
    let untrusted = install(
        &orion,
        &bundle_for("untrusted", "2.0.0", Some(&stranger)),
        false,
        false,
    );
    assert!(untrusted.detail.starts_with("-1:"), "{}", untrusted.detail);
    assert_eq!(
        list(&orion)["slots"].as_array().expect("slots").len(),
        1,
        "nothing left behind"
    );

    let good = install(
        &orion,
        &bundle_for("trusted", "2.0.0", Some(&trusted)),
        false,
        false,
    );
    assert_eq!(good.outcome, Outcome::Succeeded, "{}", good.detail);
    assert_eq!(slot(&list(&orion), "trusted")["signed"], "trusted");
}

/// A bundle whose slot.json is `manifest` after `change`, with the rest of the bundle as `bundle_for`.
fn bundle_with(id: &str, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let rootfs = format!("rootfs of {id}\n").into_bytes();
    let boot = [("cmdline.txt", &b"console=ttyS0\n"[..])];
    let mut manifest = bundle::manifest(id, "2.0.0", &rootfs, &boot);
    change(&mut manifest);
    bundle::build(&manifest, None, &rootfs, &boot)
}

#[test]
fn a_slot_for_another_board_is_refused_before_anything_is_kept() {
    let node = start("board-mismatch");
    let orion = node.connect("board-mismatch").expect("a session");
    let other = bundle_with("slot-2", |manifest| {
        manifest["board"] = Value::String("another-board".into());
    });
    let finished = install(&orion, &other, false, false);
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
    assert!(
        finished.detail.contains("another-board"),
        "{}",
        finished.detail
    );
    assert_eq!(list(&orion)["slots"].as_array().expect("slots").len(), 1);

    // A slot with no board at all is refused too.
    let none = bundle_with("slot-3", |manifest| {
        manifest.as_object_mut().expect("an object").remove("board");
    });
    let finished = install(&orion, &none, false, false);
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
}

#[test]
fn a_schema_2_slot_is_refused() {
    let node = start("schema-2");
    let orion = node.connect("schema-2").expect("a session");
    let old = bundle_with("slot-2", |manifest| {
        manifest["schema"] = Value::from(2);
        manifest
            .as_object_mut()
            .expect("an object")
            .remove("product");
        manifest["family"] = Value::String("simulated".into());
    });
    let finished = install(&orion, &old, false, false);
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
    assert!(finished.detail.contains("schema 2"), "{}", finished.detail);
}

#[test]
fn a_slot_for_this_board_lists_its_product_and_board_and_a_foreign_one_cannot_be_made_default() {
    let node = start("product-board");
    let orion = node.connect("product-board").expect("a session");
    let ours = bundle_with("slot-2", |manifest| {
        manifest["product"] = Value::String("helios".into());
        manifest["version"] = Value::String("2026.4.0".into());
    });
    assert_eq!(
        install(&orion, &ours, false, false).outcome,
        Outcome::Succeeded
    );
    let listed = slot(&list(&orion), "slot-2").clone();
    assert_eq!(listed["product"], "helios");
    assert_eq!(listed["board"], "simulated");
    assert_eq!(listed["version"], "2026.4.0");
    assert!(listed.get("family").is_none(), "family is gone from list");

    // A slot already on this board can be made default; one built for another board cannot. The
    // mismatch is put on the table directly, as a slot left on disk from another board would be.
    node.model()
        .slots
        .entries
        .iter_mut()
        .find(|entry| entry.id == "slot-2")
        .expect("slot-2")
        .board = "another-board".into();
    let listing = list(&orion);
    assert_eq!(slot(&listing, "slot-2")["board"], "another-board");
    let refused_default = refused(&orion, names::SLOTS_SET_DEFAULT, b"slot-2 false");
    assert!(
        refused_default.detail.contains("another-board"),
        "{}",
        refused_default.detail
    );
    refused(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
}

#[test]
fn an_install_refused_for_room_says_so() {
    let node = start("room");
    let orion = node.connect("room").expect("a session");
    node.model().slots.free_bytes = 100;
    let finished = install(&orion, &bundle_for("slot-2", "2.0.0", None), false, false);
    assert_ne!(finished.outcome, Outcome::Succeeded);
    assert!(finished.detail.starts_with("-28:"), "{}", finished.detail);
}

#[test]
fn readings_come_with_their_units_on_a_topic() {
    let node = start("readings");
    let orion = node.connect("reader").expect("a session");
    let mut subscriber = orion.subscribe(names::READINGS).expect("a subscription");
    let temperature = wait_for("a temperature sample", || {
        subscriber.wait(POLL).expect("the topic waits");
        subscriber
            .latest(FieldId(0))
            .map(|sample| f64::from_le_bytes(sample.payload.try_into().expect("8 bytes")))
    });
    assert!(
        (40.0..=50.0).contains(&temperature),
        "temperature {temperature}"
    );
    let mut hardware = orion
        .subscribe(names::HARDWARE)
        .expect("the hardware topic");
    let description = wait_for("the hardware description", || {
        hardware.wait(POLL).expect("the topic waits");
        hardware
            .latest(FieldId(0))
            .map(|sample| sample.payload.to_vec())
    });
    assert!(
        text(&description).contains("model="),
        "{}",
        text(&description)
    );
}

#[test]
fn a_file_dropped_mid_transfer_resumes_and_lands_whole() {
    let node = start("files");
    let data: Vec<u8> = (0..300_000_u32).map(|i| (i % 251) as u8).collect();
    let destination = "images/system.img";

    // The link cuts the first connection after 100 kB of the transfer.
    node.script().push_link(LinkFault::Drop {
        after_bytes: 100_000,
    });
    let first = node
        .connect_through_link("uploader")
        .expect("a session through the link");
    let cut = first
        .files()
        .push(destination, &mut Cursor::new(&data), |_, _| {});
    assert!(cut.is_err(), "the cut link fails the push");
    assert!(
        node.shelf()
            .committed
            .lock()
            .unwrap()
            .get(destination)
            .is_none()
    );

    // A fresh connection resumes from where the board has the bytes.
    let second = node
        .connect_through_link("uploader")
        .expect("a session through the link");
    let transfer = second
        .files()
        .push(destination, &mut Cursor::new(&data), |_, _| {})
        .expect("the resumed push");
    assert_eq!(transfer.received, data.len() as u64);
    let committed = node.shelf().committed.lock().unwrap();
    assert_eq!(committed.get(destination), Some(&data));
}

#[test]
fn a_four_mebibyte_bundle_installs_whole() {
    // The one test that moves a large bundle through an install at its own pace.
    let node = start("large-bundle");
    let orion = node.connect("large").expect("a session");
    let big = big_bundle("slot-2");
    let finished = install(&orion, &big, false, false);
    assert_eq!(finished.outcome, Outcome::Succeeded, "{}", finished.detail);
    assert_eq!(list(&orion)["state"], "idle");
}

#[test]
fn a_stalled_link_holds_then_carries_on() {
    let node = start("link-stall");
    let pause = Duration::from_millis(300);
    node.script().push_link(LinkFault::Stall { pause });
    let started = Instant::now();
    let orion = node
        .connect_through_link("slow")
        .expect("a session through the link");
    let finished = call(&orion, names::STATUS, b"").expect("an answer after the stall");
    assert_eq!(finished.outcome, Outcome::Succeeded);
    assert!(started.elapsed() >= pause, "the stall held the connection");
}

/// The three kinds of node the sim offers, and what each one has. A test builds each with
/// `SimNode::start_with(dir, &kind, port)`: `Kind::default()` (or `start(dir)`) is a node with Lemnos,
/// `Kind::Plain` a plain Orion node, and `stop_lemnos` on a node with Lemnos is the case where Lemnos
/// stops answering while the node stays up.
#[test]
fn a_plain_orion_node_has_no_lemnos_facts_or_actions_but_keeps_its_own_services() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sim-plain");
    let _ = std::fs::remove_dir_all(&dir);
    let node = SimNode::start_with(&dir, &Kind::Plain, None).expect("the plain node starts");
    let orion = node.connect("plain").expect("a session");

    // Its identity is the key fingerprint of its hello document; there are no hardware facts
    // and no offers.
    let document = orion.node().expect("the hello document");
    assert!(
        !document.fingerprint.is_empty(),
        "the fingerprint is its identity"
    );
    for fact in ["family", "serial", "state", "model", "offers"] {
        assert!(
            document.facts.iter().all(|found| found.name != fact),
            "a plain node publishes no {fact} fact"
        );
    }

    // No Lemnos actions: the slot surface is not on the node at all, so a call is not found.
    match call(&orion, names::SLOTS_LIST, b"") {
        Err(_) => {}
        Ok(finished) => assert_ne!(
            finished.outcome,
            Outcome::Succeeded,
            "a plain node has no slot list: {}",
            finished.detail
        ),
    }

    // Orion's own services work: a file pushed to it, its own file actions, and its logs topic.
    orion
        .files()
        .push("notes/hello.txt", &mut Cursor::new(&b"hi"[..]), |_, _| {})
        .expect("a file push works without Lemnos");
    assert!(
        node.shelf()
            .committed
            .lock()
            .expect("the shelf")
            .contains_key("notes/hello.txt")
    );
    ok(&orion, names::FS_WRITE, b"notes.txt 68656c6c6f");
    assert_eq!(
        ok(&orion, names::FS_READ, b"notes.txt").output,
        b"68656c6c6f"
    );
    orion.subscribe(names::LOGS).expect("the logs topic");
}

#[test]
fn a_node_whose_lemnos_stops_keeps_its_facts_and_fails_its_calls_until_it_answers_again() {
    let node = start("lemnos-stops");
    let orion = node.connect("lemnos-stops").expect("a session");
    assert_eq!(
        list(&orion)["slot_active"],
        "slot-1",
        "Lemnos answers first"
    );
    let family = hello_fact(&orion, names::HELLO_FAMILY);

    node.stop_lemnos();
    // Expected but not answering: the call fails with "not answering" (-4), not "not found".
    let stopped = call(&orion, names::SLOTS_LIST, b"").expect("the call is answered");
    assert_eq!(stopped.outcome, Outcome::Failed, "{}", stopped.detail);
    assert!(stopped.detail.starts_with("-4:"), "{}", stopped.detail);
    // The node itself stays up: its facts and its own actions still answer.
    assert_eq!(hello_fact(&orion, names::HELLO_FAMILY), family);
    ok(&orion, names::STATUS, b"");

    node.resume_lemnos();
    assert_eq!(
        list(&orion)["slot_active"],
        "slot-1",
        "Lemnos answers again"
    );
}

#[test]
fn the_board_is_reachable_on_the_socket_and_the_network_alike() {
    let node = start("both-ways");
    let direct = node.connect("direct").expect("a session on the socket");
    let network = node
        .connect_through_link("network")
        .expect("a session through the link");
    let a = call(&direct, names::STATUS, b"").expect("status");
    let b = call(&network, names::STATUS, b"").expect("status");
    assert_eq!(a.outcome, Outcome::Succeeded);
    assert_eq!(text(&a.output), text(&b.output));
}
