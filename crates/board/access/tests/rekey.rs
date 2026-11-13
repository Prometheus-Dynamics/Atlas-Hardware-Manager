//! What happens today when a node comes back under the same address with a DIFFERENT node key.
//!
//! The session connects without a key pin and keeps whatever signed hello the node gives it, so
//! the monitor reports the new node as found. Nothing in the session compares it with the key it
//! had before. The terminal layer refuses only when the caller pins the key it knew.

#![cfg(target_os = "linux")]

mod common;

use atlas_access::{AccessError, OpenRequest, ShellKind, open};
use atlas_session::Presence;
use atlas_sim_node::access::ShellServer;
use atlas_sim_node::{DEADLINE, Mode};
use common::{
    SimCarrier, connected, fresh, handle_for, lost, scratch, sessions, start_node, stated,
};

fn found_key(presence: Presence) -> Option<[u8; 32]> {
    match presence {
        Presence::Found(node) => Some(node.public_key),
        _ => None,
    }
}

#[tokio::test]
async fn a_node_back_under_the_same_address_with_another_key_is_accepted_as_the_new_node() {
    let dir_a = scratch("rekey-a");
    fresh(&dir_a);
    let node_a = start_node(&dir_a, Mode::Running, None);
    let port = node_a.network().port();
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, port);
    connected(&handle).await;
    stated(&handle).await;
    let key_a = found_key(handle.presence()).expect("the first node is found");

    // The node goes; a different node comes back on the same address.
    drop(node_a);
    lost(&handle).await;
    let dir_b = scratch("rekey-b");
    fresh(&dir_b);
    let node_b = start_node(&dir_b, Mode::Running, Some(port));
    let mut presence = handle.watch();
    let key_b = tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|p| found_key(p.clone()).is_some_and(|key| key != key_a)),
    )
    .await
    .expect("the session reports a node again")
    .expect("the monitor is running");
    let key_b = found_key(key_b.clone()).expect("found");
    assert_ne!(key_a, key_b, "a different key came back");
    // Today: no refusal and no Lost in between that holds it. The session says Found with B's key.
    assert!(matches!(handle.presence(), Presence::Found(_)));

    // The terminal layer: a caller that knew A's key is refused, and nothing is asked of B.
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let mut pinned = OpenRequest::new(80, 24);
    pinned.expected_key = Some(key_a);
    let refused = open(&handle, carrier.clone(), pinned).await;
    assert!(
        matches!(refused, Err(AccessError::KeyChanged { .. })),
        "a pinned key is checked: {refused:?}"
    );
    assert_eq!(carrier.opens.load(std::sync::atomic::Ordering::SeqCst), 0);

    // A caller that did not pin a key gets a shell on B, silently.
    let term = open(&handle, carrier.clone(), OpenRequest::new(80, 24))
        .await
        .expect("an unpinned caller gets a shell on the new node");
    assert_eq!(term.identity().public_key, key_b);
    assert_eq!(term.kind(), ShellKind::Normal);
    drop(node_b);
}
