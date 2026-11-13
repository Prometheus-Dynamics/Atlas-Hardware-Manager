//! The hello facts the simulated node publishes as Lemnos P1c defines them: an absent fact is not
//! published, a value that changes is published as it changes on the same session, `offers` is
//! live, and a fact that appears republishes the node's session.

#![cfg(target_os = "linux")]

use std::path::Path;
use std::time::Instant;

use atlas_sim_node::node::call;
use atlas_sim_node::{DEADLINE, Hello, Kind, NODE_NAME, SimNode, names};
use orion::Orion;
use orion::actions::Outcome;

/// Polls until `check` gives an answer, or the deadline runs out.
fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The text of one hello fact in the node's document, if the document has it.
fn fact_in(orion: &Orion, fact: &str) -> Option<String> {
    let document = orion.node().expect("the node document");
    document
        .facts
        .iter()
        .find(|found| found.name == fact)
        .map(|found| String::from_utf8_lossy(&found.value).into_owned())
}

fn start(name: &str, hello: Hello) -> SimNode {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("identity-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    SimNode::start_with(&dir, &Kind::WithLemnos(hello), None).expect("the node starts")
}

#[test]
fn a_fact_the_device_does_not_give_is_not_published() {
    let node = start(
        "absent",
        Hello {
            serial: String::new(),
            model: String::new(),
            ..Hello::default()
        },
    );
    let orion = node.connect("absent").expect("a session");
    assert_eq!(fact_in(&orion, "family").as_deref(), Some("simulated"));
    assert_eq!(fact_in(&orion, "serial"), None, "absent, not empty");
    assert_eq!(fact_in(&orion, "model"), None, "absent, not empty");
    let document = orion.node().expect("the document");
    assert!(
        document.facts.iter().all(|fact| !fact.value.is_empty()),
        "no empty fact"
    );
}

#[test]
fn a_node_publishes_no_name_fact_the_document_name_is_its_name() {
    let node = start("name", Hello::default());
    let orion = node.connect("name").expect("a session");
    let document = orion.node().expect("the document");
    assert_eq!(document.name, NODE_NAME);
    assert!(
        document
            .facts
            .iter()
            .all(|fact| fact.name != "name" && fact.name != "revision"),
        "name and revision are fields of the document, not facts"
    );
}

#[test]
fn a_value_that_changes_is_published_on_the_same_session() {
    let node = start("change", Hello::default());
    let orion = node.connect("change").expect("a session");
    assert_eq!(fact_in(&orion, "version.os").as_deref(), Some("1.0.0"));

    node.change_hello(|hello| hello.os_version = "1.0.1".into());
    wait_for("the new os version", || {
        (fact_in(&orion, "version.os").as_deref() == Some("1.0.1")).then_some(())
    });
    // The same session still answers: nothing was published again.
    assert_eq!(fact_in(&orion, "model").as_deref(), Some("simulated-board"));
}

#[test]
fn offers_are_live_sorted_and_space_separated() {
    let node = start("offers", Hello::default());
    let orion = node.connect("offers").expect("a session");
    assert_eq!(
        fact_in(&orion, "offers").as_deref(),
        Some("system.identity/1 system.reboot/1 system.slots/1")
    );

    node.change_hello(|hello| {
        hello.offers = vec![
            "system.slots/1".into(),
            "system.reboot/1".into(),
            "system.slots/1".into(),
        ];
    });
    wait_for("the new offers", || {
        (fact_in(&orion, "offers").as_deref() == Some("system.reboot/1 system.slots/1"))
            .then_some(())
    });
}

#[test]
fn a_fact_that_appears_republishes_the_session_and_the_node_answers_again() {
    let node = start(
        "appears",
        Hello {
            lemnos_version: String::new(),
            ..Hello::default()
        },
    );
    let before = node.connect("before").expect("a session");
    assert_eq!(fact_in(&before, "version.lemnos"), None);

    node.change_hello(|hello| hello.lemnos_version = "1.0.0".into());
    // A new session says the fact, and the node's actions answer on it.
    let after = wait_for("the new fact", || {
        let orion = node.connect("after").ok()?;
        fact_in(&orion, "version.lemnos").map(|_| orion)
    });
    assert_eq!(fact_in(&after, "version.lemnos").as_deref(), Some("1.0.0"));
    let status = call(&after, names::STATUS, b"").expect("status answers");
    assert_eq!(status.outcome, Outcome::Succeeded);
}

#[test]
fn a_plain_node_publishes_no_hello_facts_but_the_hello_document_still_answers() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("identity-plain");
    let _ = std::fs::remove_dir_all(&dir);
    let node = SimNode::start_with(&dir, &Kind::Plain, None).expect("the node starts");
    let orion = node.connect("plain").expect("a session");
    let document = orion.node().expect("the document");
    assert!(document.facts.is_empty(), "{:?}", document.facts);
}
