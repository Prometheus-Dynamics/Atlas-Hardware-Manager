//! The default sim offers what Lemnos's bridge at a6251ecf offers, on the bridge's targets, and refuses
//! the rest with Orion's and the bridge's own words. These are the fail-before tests for the call-target
//! fix: on the old sim the old node names answer and the device forms are not registered.
#![cfg(target_os = "linux")]

use atlas_sim_node::{Kind, NODE_NAME, SimNode};
use orion::Orion;
use orion::actions::{ActionCall, Finished, Outcome, Target};
use orion::types::{NodeName, ResourceName};

fn start(name: &str) -> SimNode {
    let dir = atlas_sim_node::socket::node_dir(name).expect("a node directory");
    SimNode::start_owned(dir, &Kind::default(), None).expect("the simulated node starts")
}

/// One run, as Orion answers it: its finished outcome, or the refusal Orion made before the bridge saw it.
fn run(orion: &Orion, name: &str, target: Target, args: &[u8]) -> Result<Finished, String> {
    let mut run = orion
        .run(&ActionCall {
            name,
            target,
            args,
            deadline_ms: 0,
            detached: false,
        })
        .map_err(|error| error.to_string())?;
    run.finish(atlas_sim_node::DEADLINE)
        .map_err(|error| error.to_string())
}

fn node_target() -> Target {
    Target::Node(NODE_NAME.parse::<NodeName>().expect("the node's name"))
}

fn resource(name: &str) -> Target {
    Target::Resource(name.parse::<ResourceName>().expect("a resource name"))
}

/// Asserts a run was not answered by anything: Orion's own "not found", whether it came as a refusal of
/// the run or as its ending.
fn assert_not_found(result: Result<Finished, String>, what: &str) {
    match result {
        Err(error) => assert!(error.contains("not found"), "{what}: {error}"),
        Ok(finished) => {
            assert_eq!(finished.outcome, Outcome::Rejected, "{what}");
            assert!(
                finished.detail.contains("not found"),
                "{what}: {}",
                finished.detail
            );
        }
    }
}

#[test]
fn the_old_node_names_are_not_found() {
    let node = start("bridge-surface-old-names");
    let orion = node.connect("bridge-old").expect("a session");
    for name in [
        "system.slots.list",
        "system.slots.set-default",
        "system.slots.install-file",
        "system.reboot.reboot",
        "system.flash.verify",
    ] {
        assert_not_found(run(&orion, name, node_target(), b""), name);
    }
}

#[test]
fn a_device_name_on_the_wrong_resource_is_not_found() {
    let node = start("bridge-surface-wrong-target");
    let orion = node.connect("bridge-target").expect("a session");
    assert_not_found(
        run(
            &orion,
            "lemnos.slots.call",
            resource("lemnos.reboot"),
            b"system.slots/1:list",
        ),
        "lemnos.slots.call on lemnos.reboot",
    );
    assert_not_found(
        run(
            &orion,
            "lemnos.slots.call",
            node_target(),
            b"system.slots/1:list",
        ),
        "lemnos.slots.call on the node",
    );
}

#[test]
fn the_bridge_form_on_the_device_answers() {
    let node = start("bridge-surface-answers");
    let orion = node.connect("bridge-answers").expect("a session");
    let list = run(
        &orion,
        "lemnos.slots.call",
        resource("lemnos.slots"),
        b"system.slots/1:list",
    )
    .expect("list answers");
    assert_eq!(list.outcome, Outcome::Succeeded, "{}", list.detail);
    let text = String::from_utf8_lossy(&list.output);
    assert!(text.contains("\"slots\""), "the slot list: {text}");
    let short = run(
        &orion,
        "lemnos.slots.call",
        resource("lemnos.slots"),
        b"list",
    )
    .expect("the bare name answers");
    assert_eq!(short.outcome, Outcome::Succeeded, "{}", short.detail);
    let reboot = run(
        &orion,
        "lemnos.reboot.call",
        resource("lemnos.reboot"),
        b"system.reboot/1:reboot",
    )
    .expect("reboot is a run");
    // The reboot is a restart: its answer is what the node gives, not a refusal of the words.
    assert_ne!(reboot.detail, "no method \"system.reboot/1:reboot\"");
}

#[test]
fn describe_is_the_bare_word_and_names_the_contracts() {
    let node = start("bridge-surface-describe");
    let orion = node.connect("bridge-describe").expect("a session");
    let text = run(
        &orion,
        "lemnos.slots.call",
        resource("lemnos.slots"),
        b"describe",
    )
    .expect("describe");
    assert_eq!(text.outcome, Outcome::Succeeded, "{}", text.detail);
    let body = String::from_utf8_lossy(&text.output).into_owned();
    assert!(body.contains("tag=\"system.slots/1\""), "{body}");
    let with_args = run(
        &orion,
        "lemnos.slots.call",
        resource("lemnos.slots"),
        b"describe x",
    )
    .expect("refused");
    assert_eq!(with_args.outcome, Outcome::Rejected);
    assert_eq!(with_args.detail, "describe takes no arguments");
}

#[test]
fn the_bridges_refusals_are_its_words() {
    let node = start("bridge-surface-refusals");
    let orion = node.connect("bridge-refusals").expect("a session");
    let no_method = run(
        &orion,
        "lemnos.slots.call",
        resource("lemnos.slots"),
        b"system.reboot/1:list",
    )
    .expect("refused");
    assert_eq!(no_method.outcome, Outcome::Rejected);
    assert_eq!(no_method.detail, "no method \"system.reboot/1:list\"");
    let no_words =
        run(&orion, "lemnos.slots.call", resource("lemnos.slots"), b"").expect("refused");
    assert_eq!(no_words.outcome, Outcome::Rejected);
    assert_eq!(no_words.detail, "call needs a method");
    let power = run(
        &orion,
        "lemnos.slots.power",
        resource("lemnos.slots"),
        b"maybe",
    )
    .expect("refused");
    assert_eq!(power.outcome, Outcome::Rejected);
    assert_eq!(power.detail, "power takes on or off");
    let item = run(
        &orion,
        "lemnos.slots.set",
        resource("lemnos.slots"),
        b"nothing 1",
    )
    .expect("refused");
    assert_eq!(item.outcome, Outcome::Rejected);
    assert_eq!(item.detail, "no item \"nothing\"");
}
