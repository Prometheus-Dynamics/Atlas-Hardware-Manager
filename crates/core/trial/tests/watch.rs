//! The watch, driven by events and by the board's records as text, with no clock and no board.

use atlas_inventory::Health;
use atlas_slots::{BoardOutcome, Reported, SlotList, TrialLast};
use atlas_trial::{
    Next, Outcome, Read, Running, Seen, TrialState, Watch, health_while_settling, hex,
    key_from_hex, settling,
};

const BOARD: [u8; 32] = [7; 32];
const OTHER: [u8; 32] = [9; 32];

fn slot_json(id: &str, signature: Option<&str>) -> String {
    let signature = signature.map_or(String::new(), |word| format!(r#","signature":"{word}""#));
    format!(
        r#"{{"id":"{id}","product":"p","board":"b","version":"2","variant":"","kernel":"","display_name":"","good":"untried","signed":"unsigned","signer":"","size":1,"verity":false,"flags":[],"pinned":false,"failures":0,"failed":false{signature}}}"#
    )
}

/// A slot list with `running` as the board's running slot, and the given trial and resolved records
/// (JSON text, `null` for none).
fn list(running: &str, trial: &str, resolved: &str, signature: Option<&str>) -> SlotList {
    let text = format!(
        r#"{{"slot_active":"{running}","slot_default":"slot-1","slot_previous":"","slot_trial":"","slot_staged":"","state":"idle","error":"","free_bytes":0,"reserve_bytes":0,"keep":0,"slots":[{},{}],"trial":{trial},"resolved":{resolved}}}"#,
        slot_json("slot-1", None),
        slot_json("slot-2", signature),
    );
    SlotList::parse(&text).expect("the list parses")
}

const NO_TRIAL: &str = "null";
const NO_RESOLVED: &str = "null";

fn pending(attempts: usize, last: &str) -> String {
    let attempts: Vec<String> = (1..=attempts)
        .map(|n| format!(r#"{{"n":{n},"time":"T+{n}s"}}"#))
        .collect();
    format!(
        r#"{{"slot":"slot-2","attempts_left":{},"attempts":[{}],"last":"{last}","last_time":"T+9s","reason":"","record_error":null}}"#,
        3 - attempts.len(),
        attempts.join(",")
    )
}

fn resolved(outcome: &str, by: &str, reason: &str) -> String {
    format!(
        r#"{{"slot":"slot-2","outcome":"{outcome}","by":"{by}","caller":null,"reason":"{reason}","time":"T+42s"}}"#
    )
}

fn found(running: &str) -> Seen {
    Seen::Found {
        key: BOARD,
        running: Some(running.to_owned()),
    }
}

/// The watch after the board came back, with `read` answered.
fn back(watch: &mut Watch, running: &str, read: Read) {
    assert_eq!(watch.seen(found(running)), Next::ReadRecords);
    watch.read(read);
}

#[test]
fn a_started_trial_is_restarting_until_the_board_is_seen_go_and_come_back() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    assert!(matches!(watch.state(), TrialState::Restarting { slot } if slot == "slot-2"));
    assert_eq!(watch.seen(Seen::Away), Next::Wait);
    assert!(matches!(watch.state(), TrialState::NotBackYet { .. }));
    assert!(watch.seen_away());
    assert!(
        !watch.finished(),
        "a board that is away is not a fault and not an outcome"
    );
}

#[test]
fn a_board_that_is_back_settles_on_its_trial_record_and_says_it_runs_the_trial() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    let trial = pending(1, "daemon-reached-not-yet-confirmed");
    back(
        &mut watch,
        "slot-2",
        Read::List(list("slot-2", &trial, NO_RESOLVED, None)),
    );
    let TrialState::Settling(settling) = watch.state() else {
        panic!("expected settling, got {:?}", watch.state());
    };
    assert_eq!(settling.attempts_left, Some(2));
    assert_eq!(settling.attempts.len(), 1);
    assert_eq!(settling.last, Some(TrialLast::DaemonReachedNotYetConfirmed));
    assert_eq!(settling.running, Running::Trial);
    assert!(!watch.finished());
}

#[test]
fn a_board_back_on_the_old_slot_is_shown_as_running_the_other_one() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    let trial = pending(1, "power-lost-before-daemon");
    back(
        &mut watch,
        "slot-1",
        Read::List(list("slot-1", &trial, NO_RESOLVED, None)),
    );
    let TrialState::Settling(settling) = watch.state() else {
        panic!("expected settling");
    };
    assert_eq!(settling.running, Running::Other("slot-1".into()));
    assert_eq!(settling.last, Some(TrialLast::PowerLostBeforeDaemon));
}

#[test]
fn several_power_cycles_before_the_outcome_are_followed_without_a_fault() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    for cycle in 1..=2 {
        watch.seen(Seen::Away);
        assert!(matches!(watch.state(), TrialState::NotBackYet { .. }));
        let trial = pending(cycle, "daemon-reached-not-yet-confirmed");
        back(
            &mut watch,
            "slot-2",
            Read::List(list("slot-2", &trial, NO_RESOLVED, None)),
        );
        assert!(matches!(watch.state(), TrialState::Settling(_)));
        assert!(!watch.finished());
    }
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-2",
        Read::List(list(
            "slot-2",
            NO_TRIAL,
            &resolved(
                "confirmed",
                "auto",
                "healthy for 30 s since the daemon started",
            ),
            None,
        )),
    );
    let TrialState::Held(decided) = watch.state() else {
        panic!("expected held, got {:?}", watch.state());
    };
    assert_eq!(decided.by, "auto");
    assert_eq!(decided.reason, "healthy for 30 s since the daemon started");
    assert_eq!(decided.time, "T+42s");
    assert!(watch.finished());
}

#[test]
fn a_fallen_back_trial_says_the_boards_reason_and_which_version_runs_now() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-1",
        Read::List(list(
            "slot-1",
            NO_TRIAL,
            &resolved(
                "fell-back",
                "auto",
                "the trial slot did not reach a running daemon in 3 boots",
            ),
            None,
        )),
    );
    let TrialState::DidNotHold(decided) = watch.state() else {
        panic!("expected did not hold, got {:?}", watch.state());
    };
    assert_eq!(
        decided.reason,
        "the trial slot did not reach a running daemon in 3 boots"
    );
    assert_eq!(decided.running, Running::Other("slot-1".into()));
    assert!(watch.finished());
}

#[test]
fn a_refused_signature_is_carried_into_did_not_hold() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-1",
        Read::List(list(
            "slot-1",
            NO_TRIAL,
            &resolved("fell-back", "auto", "refused"),
            Some("refused"),
        )),
    );
    assert!(watch.state().refused_by_signature());
    let TrialState::DidNotHold(decided) = watch.state() else {
        panic!("expected did not hold");
    };
    assert_eq!(decided.signature, Reported::Known(BoardOutcome::Refused));
}

#[test]
fn an_absent_signature_word_is_not_reported_and_never_refused() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-1",
        Read::List(list(
            "slot-1",
            NO_TRIAL,
            &resolved("fell-back", "auto", "x"),
            None,
        )),
    );
    assert!(!watch.state().refused_by_signature());
    let TrialState::DidNotHold(decided) = watch.state() else {
        panic!("expected did not hold");
    };
    assert_eq!(decided.signature, Reported::NotReported);
}

#[test]
fn a_confirm_record_while_the_board_runs_another_slot_shows_both_facts_and_picks_neither() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-1",
        Read::List(list(
            "slot-1",
            NO_TRIAL,
            &resolved("confirmed", "auto", "ok"),
            None,
        )),
    );
    assert_eq!(
        watch.state(),
        &TrialState::Disagree {
            slot: "slot-2".into(),
            record: Outcome::Confirmed,
            running: "slot-1".into(),
        }
    );
    assert!(watch.finished());
}

#[test]
fn a_fell_back_record_while_the_board_runs_the_trial_shows_both_facts() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-2",
        Read::List(list(
            "slot-2",
            NO_TRIAL,
            &resolved("fell-back", "auto", "x"),
            None,
        )),
    );
    assert_eq!(
        watch.state(),
        &TrialState::Disagree {
            slot: "slot-2".into(),
            record: Outcome::FellBack,
            running: "slot-2".into(),
        }
    );
}

#[test]
fn a_board_that_came_back_with_no_records_is_could_not_tell_and_stays_watched() {
    // An older board: its list has neither `trial` nor `resolved`.
    let older = SlotList::parse(r#"{"slot_active":"slot-1","slot_default":"slot-1","slots":[]}"#)
        .expect("an older list parses");
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(&mut watch, "slot-1", Read::List(older));
    assert_eq!(
        watch.state(),
        &TrialState::NoRecord {
            slot: "slot-2".into(),
            running: Running::Other("slot-1".into()),
        }
    );
    assert!(!watch.finished(), "a later return reads the records again");
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-2",
        Read::List(list(
            "slot-2",
            NO_TRIAL,
            &resolved("confirmed", "call", "by hand"),
            None,
        )),
    );
    assert!(matches!(watch.state(), TrialState::Held(_)));
}

#[test]
fn a_resolved_record_for_another_slot_is_no_record_for_this_trial() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    let other = r#"{"slot":"slot-9","outcome":"confirmed","by":"auto","caller":null,"reason":"","time":"T+1s"}"#;
    back(
        &mut watch,
        "slot-2",
        Read::List(list("slot-2", NO_TRIAL, other, None)),
    );
    assert!(matches!(watch.state(), TrialState::NoRecord { .. }));
}

#[test]
fn a_pending_trial_of_another_slot_is_could_not_tell_with_the_boards_words() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    let trial = pending(1, "power-lost-before-daemon").replace("slot-2", "slot-3");
    back(
        &mut watch,
        "slot-2",
        Read::List(list("slot-2", &trial, NO_RESOLVED, None)),
    );
    let TrialState::Unreadable { words, .. } = watch.state() else {
        panic!("expected unreadable, got {:?}", watch.state());
    };
    assert!(words.contains("slot-3") && words.contains("slot-2"));
}

#[test]
fn an_outcome_word_this_build_does_not_read_is_could_not_tell() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    let odd = r#"{"slot":"slot-2","outcome":"maybe","by":"auto","caller":null,"reason":"","time":"T+1s"}"#;
    back(
        &mut watch,
        "slot-2",
        Read::List(list("slot-2", NO_TRIAL, odd, None)),
    );
    assert!(matches!(watch.state(), TrialState::Unreadable { .. }));
    assert!(!watch.finished());
}

#[test]
fn a_refused_read_keeps_the_boards_words_and_keeps_watching() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    back(
        &mut watch,
        "slot-1",
        Read::Refused("this board has no slots".into()),
    );
    assert_eq!(
        watch.state(),
        &TrialState::Unreadable {
            slot: "slot-2".into(),
            words: "this board has no slots".into(),
        }
    );
    assert!(!watch.finished());
}

#[test]
fn a_board_that_leaves_during_a_read_waits_again_and_is_not_a_fault() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    assert_eq!(watch.seen(found("slot-2")), Next::ReadRecords);
    watch.read(Read::Gone("the node is not connected".into()));
    assert!(matches!(watch.state(), TrialState::NotBackYet { .. }));
    assert!(!watch.finished());
}

#[test]
fn a_replaced_board_ends_the_watch_with_the_reason_and_reads_nothing_from_the_other() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    assert_eq!(
        watch.seen(Seen::Replaced {
            expected: BOARD,
            found: Some(OTHER),
        }),
        Next::Wait
    );
    assert_eq!(
        watch.state(),
        &TrialState::Replaced {
            expected: hex(&BOARD),
            found: Some(hex(&OTHER)),
        }
    );
    assert!(watch.finished());
    // Nothing further from either board changes the ended watch.
    assert_eq!(watch.seen(found("slot-2")), Next::Wait);
    watch.read(Read::List(list(
        "slot-2",
        NO_TRIAL,
        &resolved("confirmed", "auto", "x"),
        None,
    )));
    assert!(matches!(watch.state(), TrialState::Replaced { .. }));
}

#[test]
fn a_found_key_other_than_the_trials_board_is_a_replacement_too() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    assert_eq!(
        watch.seen(Seen::Found {
            key: OTHER,
            running: Some("slot-2".into()),
        }),
        Next::Wait
    );
    assert!(matches!(watch.state(), TrialState::Replaced { .. }));
}

#[test]
fn a_board_that_never_returns_keeps_its_watch_and_a_person_can_stop_it() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    for _ in 0..50 {
        watch.seen(Seen::Away);
    }
    assert!(matches!(watch.state(), TrialState::NotBackYet { slot } if slot == "slot-2"));
    assert!(!watch.finished());
    watch.stop();
    assert_eq!(
        watch.state(),
        &TrialState::Stopped {
            slot: "slot-2".into()
        }
    );
    assert!(watch.finished());
}

#[test]
fn a_resumed_watch_waits_for_its_board_and_then_reads_it() {
    let mut watch = Watch::resume(BOARD, "slot-2", 5);
    assert!(matches!(watch.state(), TrialState::NotBackYet { .. }));
    assert_eq!(watch.started_ms(), 5);
    assert_eq!(watch.board(), BOARD);
    back(
        &mut watch,
        "slot-2",
        Read::List(list(
            "slot-2",
            NO_TRIAL,
            &resolved("confirmed", "auto", "x"),
            None,
        )),
    );
    assert!(matches!(watch.state(), TrialState::Held(_)));
}

#[test]
fn the_settling_predicate_follows_the_trial_record_and_not_the_replacement() {
    let trial = pending(1, "daemon-reached-not-yet-confirmed");
    let pending_list = list("slot-2", &trial, NO_RESOLVED, None);
    let idle = list("slot-1", NO_TRIAL, NO_RESOLVED, None);
    let up = Seen::Found {
        key: BOARD,
        running: Some("slot-2".into()),
    };
    assert!(settling(Some(&pending_list), &up));
    assert!(
        settling(Some(&pending_list), &Seen::Away),
        "absence keeps it settling"
    );
    assert!(!settling(Some(&idle), &up));
    assert!(!settling(None, &up));
    let replaced = Seen::Replaced {
        expected: BOARD,
        found: None,
    };
    assert!(!settling(Some(&pending_list), &replaced));
}

#[test]
fn a_settling_device_is_never_shown_red() {
    assert_eq!(health_while_settling(Health::Fault, true), Health::Away);
    assert_eq!(health_while_settling(Health::Fault, false), Health::Fault);
    assert_eq!(health_while_settling(Health::Ok, true), Health::Ok);
    assert_eq!(health_while_settling(Health::Away, true), Health::Away);
}

#[test]
fn a_node_key_round_trips_through_its_hex_text() {
    let key = [0xab_u8; 32];
    let text = hex(&key);
    assert_eq!(text.len(), 64);
    assert_eq!(key_from_hex(&text), Some(key));
    assert_eq!(key_from_hex("zz"), None);
    assert_eq!(key_from_hex(&"g".repeat(64)), None);
}

#[test]
fn a_last_trial_word_this_build_does_not_know_is_kept_as_text() {
    let mut watch = Watch::begin(BOARD, "slot-2", 1);
    watch.seen(Seen::Away);
    let trial = pending(1, "something-new");
    back(
        &mut watch,
        "slot-2",
        Read::List(list("slot-2", &trial, NO_RESOLVED, None)),
    );
    let TrialState::Settling(settling) = watch.state() else {
        panic!("expected settling");
    };
    assert_eq!(
        settling.last,
        Some(TrialLast::Other("something-new".into()))
    );
}
