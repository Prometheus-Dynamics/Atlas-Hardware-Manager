//! The Lemnos probe asks the device that carries `system.slots/1` for its list, as the bridge takes it, and
//! reads the answer. On the old code the probe sent `system.slots.list` to the node and read "not
//! answering" on a real board; this is its fail-before test against the bridge-shaped sim.
#![cfg(target_os = "linux")]

use atlas_board::lemnos::{self, from_probe};
use atlas_sim_node::{Kind, SimNode};
use atlas_types::LemnosState;

#[test]
fn the_probe_reads_answering_on_a_board_whose_bridge_answers() {
    let dir = atlas_sim_node::socket::node_dir("lemnos-probe").expect("a node directory");
    let node =
        SimNode::start_owned(dir, &Kind::default(), None).expect("the simulated node starts");
    let orion = node.connect("probe").expect("a session");
    let state = lemnos::probe(&orion, 1).expect("the session answers");
    assert_eq!(
        state,
        LemnosState::Answering,
        "the probe on the bridge-shaped sim"
    );
}

#[test]
fn a_refused_probe_is_not_answering_with_the_reason() {
    assert_eq!(
        from_probe(orion::actions::Outcome::Failed, "not found"),
        LemnosState::NotAnswering {
            reason: "not found".into()
        }
    );
}
