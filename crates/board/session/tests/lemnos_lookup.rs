//! The Lemnos lookup: what the search for a contract says about a board. Three states are kept apart:
//! a complete answer with no device carrying the contract is final ("no such contract"); a device that
//! carried it and is withdrawn or refused is "not answering yet", with since-when; and a search that did
//! not finish is "couldn't finish asking yet". Only a complete description is kept for the session, so
//! a refusal does not stick. The node is the bridge-shaped simulated one.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use atlas_session::lemnos::{self, Lookup};
use atlas_sim_node::readings::Change;
use atlas_sim_node::{DEADLINE, Kind, SimNode};
use orion::Orion;

fn node(name: &str) -> SimNode {
    let dir = atlas_sim_node::socket::node_dir(name).expect("a node directory");
    SimNode::start_owned(dir, &Kind::default(), None).expect("the simulated node starts")
}

/// The names of the device resources the node lists now.
fn devices(orion: &Orion) -> Vec<String> {
    orion
        .resources(lemnos::RESOURCE_PREFIX)
        .expect("the node lists its resources")
        .into_iter()
        .filter(|resource| resource.kind == lemnos::RESOURCE_KIND)
        .map(|resource| resource.name)
        .collect()
}

fn until(what: &str, mut check: impl FnMut() -> bool) {
    let end = Instant::now() + DEADLINE;
    while !check() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_refused_describe_is_not_yet_and_is_not_kept_for_the_session() {
    let node = node("lookup-refused");
    let orion = node.connect("lookup").expect("a session");
    node.refuse_describe(true);
    let first = lemnos::resource_cached(&orion, 11, lemnos::SLOTS).expect("the session answers");
    assert!(
        matches!(&first, Lookup::NotYet(text) if text.starts_with("couldn't finish asking the board what it offers yet, for ")),
        "a search that every device refused is not yet: {first:?}"
    );
    // The board answers again: the same session finds the slots. A kept refusal would say they are not there.
    // The refusal is held for its backoff only: once that passes, the slots are asked for and found.
    node.refuse_describe(false);
    let mut second = Lookup::NoDevices;
    until("the slots found after the refusal's backoff", || {
        second = lemnos::resource_cached(&orion, 11, lemnos::SLOTS).expect("the session answers");
        second == Lookup::Found("lemnos.slots".into())
    });
}

#[test]
fn an_off_device_is_an_answer_and_a_device_that_comes_on_is_found_in_the_same_session() {
    let node = node("lookup-off");
    let orion = node.connect("lookup").expect("a session");
    // `off0` is disabled by its board file: it offers nothing now, so the answer is complete and final.
    let off = lemnos::resource_cached(&orion, 14, "sim.counter/1").expect("the session answers");
    assert_eq!(
        off,
        Lookup::NoContract,
        "an off device is an answer: {off:?}"
    );
    // It comes on, and its contract is found: final means for now, not for the session.
    node.change_device(
        "off0",
        Change::State {
            state: 0,
            reason: String::new(),
        },
    );
    let mut on = Lookup::NoDevices;
    until("off0 found once it is on", || {
        on = lemnos::resource_cached(&orion, 14, "sim.counter/1").expect("the session answers");
        on == Lookup::Found("lemnos.off0".into())
    });
}

#[test]
fn no_such_contract_is_final_only_from_a_complete_answer() {
    let node = node("lookup-final");
    let orion = node.connect("lookup").expect("a session");
    // The on devices refuse their descriptions for a moment: the answer is not complete, so it is not
    // "no such contract" yet. (The off device, `off0`, is an answer, and does not make it incomplete.)
    node.refuse_describe(true);
    let partial = lemnos::resource_cached(&orion, 12, lemnos::FLASH).expect("the session answers");
    assert!(
        matches!(&partial, Lookup::NotYet(text) if text.starts_with("couldn't finish asking")),
        "a refused search is not final: {partial:?}"
    );
    // Once they answer, every device has been described, and none carries the contract: final.
    node.refuse_describe(false);
    let mut complete = Lookup::NoDevices;
    until(
        "the on devices answer again, with no flash contract",
        || {
            complete =
                lemnos::resource_cached(&orion, 12, lemnos::FLASH).expect("the session answers");
            complete == Lookup::NoContract
        },
    );
}

#[test]
fn a_device_that_carried_the_contract_is_not_answering_yet_while_it_is_withdrawn() {
    let node = node("lookup-withdrawn");
    let orion = node.connect("lookup").expect("a session");
    let found = lemnos::resource_cached(&orion, 13, lemnos::SLOTS).expect("the session answers");
    assert_eq!(found, Lookup::Found("lemnos.slots".into()));

    // lemnosd abandons the device for a moment: it is not listed, and the board's slots are not there yet.
    node.hold_devices(true);
    until("every device withdrawn", || devices(&orion).is_empty());
    let gone = lemnos::resource_cached(&orion, 13, lemnos::SLOTS).expect("the session answers");
    assert!(
        matches!(&gone, Lookup::NotYet(text) if text.starts_with("the board's slots device isn't answering yet, for ")),
        "a device that carried the contract is not yet, not absent: {gone:?}"
    );

    node.hold_devices(false);
    until("the devices listed again", || {
        devices(&orion).contains(&"lemnos.slots".to_owned())
    });
    let back = lemnos::resource_cached(&orion, 13, lemnos::SLOTS).expect("the session answers");
    assert_eq!(back, Lookup::Found("lemnos.slots".into()), "{back:?}");
}

#[test]
fn the_not_yet_sentence_counts_up_on_each_lookup() {
    let node = node("lookup-counts");
    let orion = node.connect("lookup").expect("a session");
    node.refuse_describe(true);
    let first = lemnos::resource_cached(&orion, 15, lemnos::SLOTS).expect("the session answers");
    assert!(
        matches!(&first, Lookup::NotYet(text) if text.ends_with(", for 0 ms")),
        "{first:?}"
    );
    std::thread::sleep(Duration::from_millis(1100));
    let later = lemnos::resource_cached(&orion, 15, lemnos::SLOTS).expect("the session answers");
    // The sentence counts the time since the first lookup. A loaded machine sleeps longer than it
    // was asked to, so the time is read and held to "at least what was slept", not to one exact text
    // (it read "1.2 s" under load, 2026-11-23).
    let Lookup::NotYet(text) = &later else {
        panic!("{later:?}");
    };
    let seconds: f64 = text
        .rsplit_once(", for ")
        .and_then(|(_, time)| time.strip_suffix(" s"))
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("the time is in seconds: {text}"));
    assert!((1.1..30.0).contains(&seconds), "{text}");
    node.refuse_describe(false);
}
