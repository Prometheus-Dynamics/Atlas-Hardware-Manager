//! The proofs of T4: `atlas-next cycle` run through the same `execute` the binary runs, against a
//! simulated board (`atlas-sim-node`) on this machine's loopback. The board's side of each restart
//! is played by a driver that runs next to the command, as a board would.

mod common;

use atlas_sim_node::{SimNode, names};
use common::{Relay, bundle_bytes, env, identity, run, scratch, start_node, until, write_bundle};

const GOOD: &str = "slot-2";
const BAD: &str = "slot-3";

/// A board restart as the address sees it: the board leaves, comes back, and the restart's boot
/// is the one the sim now runs (`power_cut` then `reboot`, as the slot tests do).
fn restart(node: &SimNode, relay: &Relay) {
    relay.away(true);
    relay.cut();
    {
        let mut model = node.model();
        model.slots.power_cut();
        model.slots.reboot();
    }
    relay.away(false);
}

/// The board's part of the cycle, in the order the command asks for it.
///
/// - The good trial: when it starts, the board restarts into it, then the command reads its records
///   (its first list read after the trial call), then the board confirms itself after 30 s.
/// - The bad trial: the command starts it and does not follow it; the board restarts into it.
///   The command then tries the good slot again, which the board must refuse; once that refusal
///   has come (a second install job), the bad trial fails: its deadline passes with the board
///   unhealthy and it falls back to the default by itself. With `bad_boots` the bad slot is
///   healthy instead and confirms itself, which the command must report as a failure.
async fn play_board(node: &SimNode, relay: &Relay, bad_boots: bool) {
    until("the good trial to start on the board", || {
        node.model()
            .slots
            .trial
            .as_ref()
            .is_some_and(|trial| trial.id == GOOD)
    })
    .await;
    let lists_before = node.runs_started(names::SLOTS_LIST);
    until("the command to read the trial's records", || {
        node.runs_started(names::SLOTS_LIST) > lists_before
    })
    .await;
    restart(node, relay);
    until("the board to run the good trial", || {
        node.model().slots.active.as_deref() == Some(GOOD)
    })
    .await;
    node.slot_clock_advance(30);

    until("the bad trial to start on the board", || {
        node.model()
            .slots
            .trial
            .as_ref()
            .is_some_and(|trial| trial.id == BAD)
    })
    .await;
    // The refusal must be recorded before the trial can end: the install job runs after its start.
    until(
        "the board to refuse the good install while the bad trial is pending",
        || {
            node.model()
                .slots
                .last_attempt
                .as_ref()
                .is_some_and(|attempt| {
                    attempt.state == "refused" && attempt.reason.contains("is on trial with")
                })
        },
    )
    .await;
    if bad_boots {
        // Healthy: the board confirms the bad trial itself.
        node.slot_clock_advance(30);
    } else {
        node.slot_set_healthy(false);
        // The deadline passes with the board unhealthy: it falls back to the default by itself.
        node.slot_clock_advance(600);
        until("the board to fall back to the default", || {
            node.model().slots.active.as_deref() == Some(GOOD)
        })
        .await;
        relay.away(true);
        relay.cut();
        relay.away(false);
    }
}

/// Starts a board, a relay in front of it, a pin for the board, and two bundles.
fn rig(dir: &std::path::Path) -> (SimNode, Relay, std::path::PathBuf, std::path::PathBuf) {
    let node = start_node(dir, None);
    node.slot_follow_boot(true);
    let relay = Relay::start(
        format!("127.0.0.1:{}", node.network().port())
            .parse()
            .expect("an address"),
    );
    let good = write_bundle(
        &dir.join("bundles"),
        "good.tar",
        &bundle_bytes(GOOD, "2.0.0", None),
    );
    let bad = write_bundle(
        &dir.join("bundles"),
        "bad.tar",
        &bundle_bytes(BAD, "3.0.0", None),
    );
    (node, relay, good, bad)
}

fn cycle_args<'a>(
    address: &'a str,
    good: &'a str,
    bad: &'a str,
    card: &'a str,
    state: &'a str,
    identity_file: &'a str,
) -> Vec<&'a str> {
    vec![
        "cycle",
        address,
        "--good",
        good,
        "--bad",
        bad,
        "--scorecard",
        card,
        "--state",
        state,
        "--identity",
        identity_file,
    ]
}

fn text(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).expect("the scorecard is written")
}

/// Proof 1: a full run on a fresh simulated board with a good and a bad bundle. Every step passes,
/// the boot id is read and changes at each restart, the trial-pending refusal's words are the
/// sim's, and the scorecard holds every step's words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_run_passes_every_step_and_writes_the_scorecard() {
    let dir = scratch("cyc-full");
    let (node, relay, good, bad) = rig(&dir);
    let mut env = env(&dir.join("state"));
    env.info_in_pin = true;
    let address = relay.address();
    let card = dir.join("card").join("scorecard.txt");
    let state = dir.join("state").display().to_string();
    let identity_file = dir.join("state").join("identity.key").display().to_string();
    let good_path = good.display().to_string();
    let bad_path = bad.display().to_string();
    let card_path = card.display().to_string();
    let args = cycle_args(
        &address,
        &good_path,
        &bad_path,
        &card_path,
        &state,
        &identity_file,
    );

    let ((code, out), ()) = tokio::join!(run(&env, &args), play_board(&node, &relay, false));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("exit status 0"), "{out}");

    let card_text = text(&card);
    for step in [
        "Step 1: PASS",
        "Step 2: PASS",
        "Step 3: PASS",
        "Step 4: PASS",
        "Step 5: PASS",
        "Step 6: PASS",
        "Step 7a: PASS",
        "Step 7b: PASS",
        "Step 7c: PASS",
        "Step 7d: PASS",
        "Step 8a: PASS",
        "Step 8b: PASS",
    ] {
        assert!(card_text.contains(step), "{step} is missing:\n{card_text}");
    }
    assert!(
        card_text.contains("is on trial with 3 attempts left: nothing was written"),
        "the board's words for the refused install are kept:\n{card_text}"
    );
    assert!(
        card_text.contains("came back in the previous slot slot-2 by itself"),
        "{card_text}"
    );
    assert!(
        card_text.contains("the boot id changed: the board really restarted"),
        "{card_text}"
    );
    assert!(card_text.contains("Verdict: PASSED"), "{card_text}");
    assert!(
        // The sim's hello carries no security fact (the stand-in comm core has no record): the
        // board has not said. Orion's `Node::security` reads it from the hello.
        card_text.contains("Security on the board: the board has not said"),
        "{card_text}"
    );
    assert!(
        card_text.contains(&node_fingerprint(&node).await),
        "{card_text}"
    );
    assert!(
        card_text.contains("This PC's client fingerprint: sha256:"),
        "{card_text}"
    );
    assert!(
        card_text.contains("Run ended (this PC's clock, UTC): 20"),
        "{card_text}"
    );

    assert!(
        !card.with_extension("json").exists(),
        "no JSON without --json"
    );

    // The board's slots at the end: the good slot is running and default, and the bad one failed.
    let model = node.model();
    assert_eq!(model.slots.default.as_deref(), Some(GOOD));
    assert_eq!(model.slots.active.as_deref(), Some(GOOD));
    assert!(model.slots.get(BAD).is_some_and(|slot| slot.failed));
}

/// Proof 1, the same run without the boot field (the pin this build carries): the boot id lines say
/// `not checked`, the run still passes on the slot list, and nothing is read as a pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_boot_field_the_boot_id_is_recorded_as_not_checked() {
    let dir = scratch("cyc-noboot");
    let (node, relay, good, bad) = rig(&dir);
    let env = env(&dir.join("state"));
    assert!(
        !env.info_in_pin,
        "the default pin does not carry the boot id"
    );
    let address = relay.address();
    let card = dir.join("card.txt");
    let state = dir.join("state").display().to_string();
    let identity_file = dir.join("state").join("identity.key").display().to_string();
    let good_path = good.display().to_string();
    let bad_path = bad.display().to_string();
    let card_path = card.display().to_string();
    let args = cycle_args(
        &address,
        &good_path,
        &bad_path,
        &card_path,
        &state,
        &identity_file,
    );

    let ((code, out), ()) = tokio::join!(run(&env, &args), play_board(&node, &relay, false));
    assert_eq!(code, 0, "{out}");
    let card_text = text(&card);
    assert!(
        card_text.contains("not checked (needs Lemnos I5k: info.last_attempt.boot)"),
        "{card_text}"
    );
    assert!(card_text.contains("Verdict: PASSED"), "{card_text}");
    assert!(!card_text.contains("the boot id changed"), "{card_text}");
}

/// Proof 2: the bad bundle in fact boots. Its trial confirms itself, so step 7d fails: the run
/// exits 1 and the scorecard says why in words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bad_bundle_that_boots_fails_the_fallback_step_and_exits_1() {
    let dir = scratch("cyc-boots");
    let (node, relay, good, bad) = rig(&dir);
    let env = env(&dir.join("state"));
    let address = relay.address();
    let card = dir.join("card.txt");
    let state = dir.join("state").display().to_string();
    let identity_file = dir.join("state").join("identity.key").display().to_string();
    let good_path = good.display().to_string();
    let bad_path = bad.display().to_string();
    let card_path = card.display().to_string();
    let args = cycle_args(
        &address,
        &good_path,
        &bad_path,
        &card_path,
        &state,
        &identity_file,
    );

    let ((code, out), ()) = tokio::join!(run(&env, &args), play_board(&node, &relay, true));
    assert_eq!(code, 1, "{out}");
    assert!(out_has(&out, "Step 7d: FAIL"), "{out}");
    let card_text = text(&card);
    assert!(card_text.contains("Step 7d: FAIL"), "{card_text}");
    assert!(
        card_text.contains("came back in the trial slot slot-3"),
        "the scorecard says the bad slot held: {card_text}"
    );
    assert!(
        card_text.contains("did not fall back to the previous slot"),
        "{card_text}"
    );
    assert!(card_text.contains("Step 8a: NOT RUN"), "{card_text}");
    assert!(card_text.contains("STOPPED at step 7d"), "{card_text}");
}

/// Proof 3: the address answers with another key than the pinned one. The run stops at step 3 with
/// exit 3, and the scorecard holds both fingerprints. Nothing else is asked of the board.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_pinned_to_another_key_stops_at_step_3_with_exit_3() {
    let dir = scratch("cyc-pin");
    let node = start_node(&dir, None);
    let port = node.network().port();
    let address = format!("orion+tcp://127.0.0.1:{port}");
    let first = identity(&node).await;
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    drop(node);
    let replacement = start_node(&dir.join("replacement"), Some(port));
    let second = identity(&replacement).await;
    assert_ne!(first.public_key, second.public_key);

    let good = write_bundle(
        &dir.join("bundles"),
        "good.tar",
        &bundle_bytes(GOOD, "2.0.0", None),
    );
    let bad = write_bundle(
        &dir.join("bundles"),
        "bad.tar",
        &bundle_bytes(BAD, "3.0.0", None),
    );
    let card = dir.join("card.txt");
    let state = dir.join("state").display().to_string();
    let identity_file = dir.join("state").join("identity.key").display().to_string();
    let good_path = good.display().to_string();
    let bad_path = bad.display().to_string();
    let card_path = card.display().to_string();
    let args = cycle_args(
        &address,
        &good_path,
        &bad_path,
        &card_path,
        &state,
        &identity_file,
    );

    let (code, out) = run(&env, &args).await;
    assert_eq!(code, 3, "{out}");
    let card_text = text(&card);
    assert!(card_text.contains("Step 3: FAIL"), "{card_text}");
    assert!(
        card_text.contains(&first.fingerprint),
        "the pinned fingerprint: {card_text}"
    );
    assert!(
        card_text.contains(&second.fingerprint),
        "the answering fingerprint: {card_text}"
    );
    assert!(card_text.contains("reflash"), "{card_text}");
    assert!(card_text.contains("Step 4: NOT RUN"), "{card_text}");
    assert_eq!(
        replacement.runs_started(names::SLOTS_LIST),
        0,
        "nothing was read from it"
    );
    assert_eq!(
        replacement.runs_started(names::SLOTS_INSTALL_FILE),
        0,
        "nothing was installed"
    );
}

/// Proof 4: `--dry-run` sends nothing (the sim records no request, and no identity file is made),
/// lists every step with its return path, and exits 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dry_run_sends_nothing_and_lists_every_step_with_its_return_path() {
    let dir = scratch("cyc-dry");
    let node = start_node(&dir, None);
    let address = format!("orion+tcp://127.0.0.1:{}", node.network().port());
    let env = env(&dir.join("state"));
    let good = dir.join("good.tar");
    let bad = dir.join("bad.tar");
    std::fs::create_dir_all(&dir).expect("the directory is made");
    std::fs::write(&good, b"not read by a dry run").expect("the file is written");
    std::fs::write(&bad, b"not read by a dry run").expect("the file is written");
    let card = dir.join("card.txt");
    let state = dir.join("state").display().to_string();
    let identity_file = dir.join("state").join("identity.key").display().to_string();
    let good_path = good.display().to_string();
    let bad_path = bad.display().to_string();
    let card_path = card.display().to_string();
    let mut args = cycle_args(
        &address,
        &good_path,
        &bad_path,
        &card_path,
        &state,
        &identity_file,
    );
    args.push("--dry-run");

    let (code, out) = run(&env, &args).await;
    assert_eq!(code, 0, "{out}");
    assert_eq!(node.calls_received(), 0, "the board was not asked anything");
    assert!(
        !dir.join("state").join("identity.key").exists(),
        "no identity was made"
    );
    assert!(!card.exists(), "no scorecard was written");
    for step in [
        "Step 1: ",
        "Step 2: ",
        "Step 3: ",
        "Step 4: ",
        "Step 5: ",
        "Step 6: ",
        "Step 7a: ",
        "Step 7b: ",
        "Step 7c: ",
        "Step 7d: ",
        "Step 8a: ",
        "Step 8b: ",
    ] {
        assert!(out.contains(step), "{step} is listed:\n{out}");
    }
    assert!(
        out.contains("if it fails, the board comes back like this"),
        "{out}"
    );
    assert!(out.contains("Nothing is sent to any board."), "{out}");
}

/// Proof 5: the board lists the good slot as missing right after its install. The run stops at step
/// 5 with exit 1; nothing is repaired, and no trial or default change is ever sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unexpected_state_stops_the_run_at_that_step() {
    let dir = scratch("cyc-missing");
    let (node, relay, good, bad) = rig(&dir);
    node.slot_hide_from_list(Some(GOOD));
    let env = env(&dir.join("state"));
    let address = relay.address();
    let card = dir.join("card.txt");
    let state = dir.join("state").display().to_string();
    let identity_file = dir.join("state").join("identity.key").display().to_string();
    let good_path = good.display().to_string();
    let bad_path = bad.display().to_string();
    let card_path = card.display().to_string();
    let args = cycle_args(
        &address,
        &good_path,
        &bad_path,
        &card_path,
        &state,
        &identity_file,
    );

    let (code, out) = run(&env, &args).await;
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("does not show slot-2 after the install"),
        "{out}"
    );
    let card_text = text(&card);
    assert!(card_text.contains("Step 5: FAIL"), "{card_text}");
    assert!(card_text.contains("Step 6: NOT RUN"), "{card_text}");
    assert!(card_text.contains("STOPPED at step 5"), "{card_text}");
    assert_eq!(
        node.runs_started(names::SLOTS_TRIAL),
        0,
        "no trial was sent"
    );
    assert_eq!(
        node.runs_started(names::SLOTS_SET_DEFAULT),
        0,
        "no default was sent"
    );
    assert_eq!(
        node.runs_started(names::SLOTS_INSTALL_FILE),
        1,
        "one install, and no more"
    );
    assert_eq!(
        node.runs_started(names::SLOTS_LIST),
        3,
        "the list before, the install's own snapshot of the slots before its start, and the check after, and no more"
    );
}

/// The words of a run's output, where a test looks for one line.
fn out_has(out: &str, line: &str) -> bool {
    out.lines().any(|found| found.contains(line))
}

/// The board's key fingerprint, as the sim answers it now.
async fn node_fingerprint(node: &SimNode) -> String {
    identity(node).await.fingerprint
}
