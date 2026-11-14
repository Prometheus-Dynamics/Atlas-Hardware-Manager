//! The six proofs of T1, run through the same `execute` the binary runs, against a simulated
//! board (`atlas-sim-node`) on this machine's loopback.

mod common;

use atlas_next::{PINS_FILE, parse_address};
use atlas_sim_node::{SimNode, names};
use atlas_trial::hex;
use common::{Relay, bundle_bytes, env, identity, run, scratch, start_node, until, write_bundle};

fn direct(node: &SimNode) -> String {
    format!("orion+tcp://127.0.0.1:{}", node.network().port())
}

/// Proof 1: a board never seen refuses every command and shows its fingerprint; `pin` records it;
/// the commands then work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_contact_refuses_until_pinned_then_lists() {
    let dir = scratch("first");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let board = identity(&node).await;
    let env = env(&dir.join("state"));

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 3, "{out}");
    assert!(out.contains(&board.fingerprint), "{out}");
    assert!(out.contains("`atlas-next slots"), "{out}");
    assert!(out.contains(" pin`"), "{out}");
    assert_eq!(node.runs_started(names::SLOTS_LIST), 0, "nothing was asked");

    let (code, out) = run(&env, &["slots", &address, "fingerprint"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&board.fingerprint), "{out}");
    assert!(
        !dir.join("state").join(PINS_FILE).exists(),
        "fingerprint pins nothing"
    );

    let (code, out) = run(&env, &["slots", &address, "list", "--json"]).await;
    assert_eq!(code, 3, "{out}");
    let document: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON document");
    assert_eq!(document["ok"], false);
    assert_eq!(document["code"], 3);

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");
    let pins =
        std::fs::read_to_string(dir.join("state").join(PINS_FILE)).expect("the pin is written");
    assert!(pins.contains(&hex(&board.public_key)), "{pins}");
    assert!(pins.contains(&board.fingerprint), "{pins}");

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "pinning again changes nothing: {out}");

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("slot-1"), "{out}");
}

/// Proof 2: a different key on the same address refuses with both fingerprints; `repin` without
/// `--reflashed` refuses; with it, the pin is replaced and the commands work again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replaced_board_is_refused_with_both_fingerprints_until_repinned() {
    let dir = scratch("repl");
    let node = start_node(&dir, None);
    let port = node.network().port();
    let address = direct(&node);
    let first = identity(&node).await;
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    drop(node);
    let replacement = start_node(&dir.join("replacement"), Some(port));
    let second = identity(&replacement).await;
    assert_ne!(
        first.public_key, second.public_key,
        "a new board has a new key"
    );

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 3, "{out}");
    assert!(
        out.contains(&first.fingerprint),
        "the pinned fingerprint: {out}"
    );
    assert!(
        out.contains(&second.fingerprint),
        "the answering fingerprint: {out}"
    );
    assert!(out.contains("reflash"), "{out}");
    assert_eq!(
        replacement.runs_started(names::SLOTS_LIST),
        0,
        "nothing was read from it"
    );

    let (code, out) = run(&env, &["slots", &address, "repin"]).await;
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("--reflashed"), "{out}");

    let (code, out) = run(&env, &["slots", &address, "repin", "--reflashed"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&second.fingerprint), "{out}");

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("slot-1"), "{out}");
}

/// Proof 3: install a bundle, trial it, the board restarts into it and comes back, the trial is
/// reported as held, confirm makes it the default, and the list shows it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cycle_installs_trials_confirms_and_lists() {
    let dir = scratch("cycle");
    let node = start_node(&dir, None);
    node.slot_follow_boot(true);
    let relay = Relay::start(
        format!("127.0.0.1:{}", node.network().port())
            .parse()
            .expect("an address"),
    );
    let address = relay.address();
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    let bundle = write_bundle(
        &dir.join("bundles"),
        "slot-2.tar",
        &bundle_bytes("slot-2", "2.0.0", None),
    );
    let bundle = bundle.to_str().expect("a UTF-8 path").to_owned();
    let (code, out) = run(&env, &["slots", &address, "install", &bundle]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("installed as slot-2"), "{out}");

    let trial_args = ["slots", &address, "trial", "slot-2"];
    let lists_before = node.runs_started(names::SLOTS_LIST);
    let trial = run(&env, &trial_args);
    let board_restarts = async {
        // The command reads the board's records only after its trial call came back: the board
        // may leave from here on, and the call's answer was already received.
        until("the command to follow the trial", || {
            node.runs_started(names::SLOTS_LIST) > lists_before
        })
        .await;
        until("the trial to start on the board", || {
            node.model().slots.trial.is_some()
        })
        .await;
        // The board leaves (its reboot), then comes back running the trial slot.
        relay.away(true);
        relay.cut();
        {
            let mut model = node.model();
            model.slots.power_cut();
            model.slots.reboot();
        }
        until("the board to run the trial slot", || {
            node.model().slots.active.as_deref() == Some("slot-2")
        })
        .await;
        relay.away(false);
        // The board's daemon has been up long enough: it confirms itself.
        node.slot_clock_advance(30);
    };
    let ((code, out), ()) = tokio::join!(trial, board_restarts);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("came back in the trial slot slot-2"), "{out}");

    let (code, out) = run(&env, &["slots", &address, "confirm", "slot-2"]).await;
    assert_eq!(code, 0, "{out}");
    assert_eq!(node.model().slots.default.as_deref(), Some("slot-2"));

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 0, "{out}");
    let line = out
        .lines()
        .find(|line| line.starts_with("slot-2"))
        .expect("slot-2 is listed");
    assert!(
        line.contains("default") && line.contains("running"),
        "{line}"
    );

    // The running and default slot is refused, here before the board is asked.
    let (code, out) = run(&env, &["slots", &address, "remove", "slot-2"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("slot-2"), "{out}");
    // The old slot is neither running nor default now: it is removed.
    let (code, out) = run(&env, &["slots", &address, "remove", "slot-1"]).await;
    assert_eq!(code, 0, "{out}");
    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(!out.lines().any(|line| line.starts_with("slot-1")), "{out}");
}

/// Proof 4: the board comes back in the previous slot. The trial says so and exits 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_trial_says_the_board_came_back_in_the_previous_slot() {
    let dir = scratch("fail");
    let node = start_node(&dir, None);
    node.slot_follow_boot(true);
    node.slot_set_healthy(false);
    let relay = Relay::start(
        format!("127.0.0.1:{}", node.network().port())
            .parse()
            .expect("an address"),
    );
    let address = relay.address();
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");
    let bundle = write_bundle(
        &dir.join("bundles"),
        "slot-2.tar",
        &bundle_bytes("slot-2", "2.0.0", None),
    );
    let bundle = bundle.to_str().expect("a UTF-8 path").to_owned();
    let (code, out) = run(&env, &["slots", &address, "install", &bundle]).await;
    assert_eq!(code, 0, "{out}");

    let trial_args = ["slots", &address, "trial", "slot-2"];
    let lists_before = node.runs_started(names::SLOTS_LIST);
    let trial = run(&env, &trial_args);
    let board_falls_back = async {
        // The command reads the board's records only after its trial call came back: the board
        // may leave from here on, and the call's answer was already received.
        until("the command to follow the trial", || {
            node.runs_started(names::SLOTS_LIST) > lists_before
        })
        .await;
        until("the trial to start on the board", || {
            node.model().slots.trial.is_some()
        })
        .await;
        // The deadline passes with the board unhealthy: it falls back and restarts.
        node.slot_clock_advance(600);
        until("the board to fall back", || {
            node.model().slots.resolved.is_some()
        })
        .await;
        relay.away(true);
        relay.cut();
        until("the board to run the previous slot", || {
            node.model().slots.active.as_deref() == Some("slot-1")
        })
        .await;
        relay.away(false);
    };
    let ((code, out), ()) = tokio::join!(trial, board_falls_back);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("came back in the previous slot: the trial failed"),
        "{out}"
    );
}

/// Proof 5: the address forms Orion reads. Bare forms (no `orion+tcp://`) are refused by Orion as
/// written; nothing is added to them here.
#[test]
fn the_address_forms_are_read_as_orion_reads_them() {
    let ok = [
        ("orion+tcp://127.0.0.1:9200", "orion+tcp://127.0.0.1:9200"),
        ("orion+tcp://[::1]:9200", "orion+tcp://[::1]:9200"),
        (
            "orion+tcp://[fe80::1%eth0]:9200",
            "orion+tcp://[fe80::1%eth0]:9200",
        ),
    ];
    for (text, shown) in ok {
        let address = parse_address(text).unwrap_or_else(|why| panic!("{text}: {why}"));
        assert_eq!(address.to_string(), shown, "{text}");
    }
    for text in ["127.0.0.1:9200", "[::1]:9200"] {
        let error = parse_address(text).expect_err("a bare address is refused");
        assert_eq!(
            error,
            format!(
                "{text:?} is not an address: a socket path, unix:<path>, orion+tcp://host[:port] or serial:<path>[@baud]"
            )
        );
    }
}

/// The command line refuses a bare address with Orion's words and exit 2, before any board is asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_address_is_a_usage_error_with_orions_words() {
    let dir = scratch("bare");
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", "127.0.0.1:9200", "list"]).await;
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("is not an address"), "{out}");
}

/// Proof 6: a bundle for another board is refused by the job; the words are printed as they are
/// and nothing is sent to the board.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_for_another_board_prints_the_refusal_and_sends_nothing() {
    let dir = scratch("refuse");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    let bundle = write_bundle(
        &dir.join("bundles"),
        "other.tar",
        &bundle_bytes("slot-9", "9.0.0", Some("other-board")),
    );
    let bundle = bundle.to_str().expect("a UTF-8 path").to_owned();
    let (code, out) = run(&env, &["slots", &address, "install", &bundle]).await;
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("other-board") && out.contains(common::BOARD),
        "{out}"
    );
    assert_eq!(
        node.runs_started(names::SLOTS_ACCEPT),
        0,
        "no accept was sent"
    );
    assert_eq!(
        node.runs_started(names::SLOTS_INSTALL_FILE),
        0,
        "no install was sent"
    );
}

/// Ctrl-C stops the following and changes nothing on the board: the trial stays pending and the
/// command says the board did not come back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_trial_leaves_the_boards_trial_pending() {
    let dir = scratch("stop");
    let node = start_node(&dir, None);
    node.slot_follow_boot(true);
    let address = direct(&node);
    let mut env = env(&dir.join("state"));
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    env.stop = Some(std::sync::Arc::clone(&stop));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");
    let bundle = write_bundle(
        &dir.join("bundles"),
        "slot-2.tar",
        &bundle_bytes("slot-2", "2.0.0", None),
    );
    let bundle = bundle.to_str().expect("a UTF-8 path").to_owned();
    let (code, out) = run(&env, &["slots", &address, "install", &bundle]).await;
    assert_eq!(code, 0, "{out}");

    let trial_args = ["slots", &address, "trial", "slot-2"];
    let trial = run(&env, &trial_args);
    let person_stops = async {
        until("the trial to start on the board", || {
            node.model().slots.trial.is_some()
        })
        .await;
        stop.notify_one();
    };
    let ((code, out), ()) = tokio::join!(trial, person_stops);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("you stopped following the trial of slot-2"),
        "{out}"
    );
    assert!(
        node.model().slots.trial.is_some(),
        "the board's trial is unchanged"
    );
}
