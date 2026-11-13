//! A node back under the same address with a DIFFERENT node key is a replacement, not the same
//! board. The session is pinned to the key it first saw: the other node is reported as
//! replaced, is sent no call, and gets no shell. A terminal open when the board is replaced ends
//! with a reason that says so.

#![cfg(target_os = "linux")]

mod common;

use std::sync::atomic::Ordering;

use atlas_access::{AccessError, End, Event, OpenRequest, open};
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
async fn a_node_back_under_the_same_address_with_another_key_is_replaced_and_gets_no_shell() {
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
    let replaced = tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|p| matches!(p, Presence::Replaced { .. })),
    )
    .await
    .expect("the session reports the replacement")
    .expect("the monitor is running")
    .clone();
    assert!(
        matches!(replaced, Presence::Replaced { expected, .. } if expected == key_a),
        "the session is pinned to the key it saw: {replaced:?}"
    );
    assert_eq!(
        handle.presence(),
        replaced,
        "a replaced node is never Found"
    );

    // No shell on the other node: the open is refused on this side and nothing reaches B.
    let carrier = SimCarrier::new(shell.addr());
    let refused = open(&handle, carrier.clone(), OpenRequest::new(80, 24)).await;
    assert!(
        matches!(refused, Err(AccessError::Gone(_))),
        "a replaced node gets no shell: {refused:?}"
    );
    assert_eq!(carrier.opens.load(Ordering::SeqCst), 0);
    assert_eq!(
        node_b.calls_received(),
        0,
        "the other node received no call"
    );
    drop(node_b);
}

#[tokio::test]
async fn a_caller_that_knows_a_different_key_is_refused_before_any_shell_opens() {
    let dir_a = scratch("pinned-a");
    fresh(&dir_a);
    let node_a = start_node(&dir_a, Mode::Running, None);
    let port = node_a.network().port();
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, port);
    connected(&handle).await;
    stated(&handle).await;
    let key_a = found_key(handle.presence()).expect("the node is found");

    let carrier = SimCarrier::new(shell.addr());
    let mut other = OpenRequest::new(80, 24);
    other.expected_key = Some([0x5a; 32]);
    let refused = open(&handle, carrier.clone(), other).await;
    assert!(
        matches!(refused, Err(AccessError::KeyChanged { .. })),
        "a key the caller did not know is refused: {refused:?}"
    );
    assert_eq!(carrier.opens.load(Ordering::SeqCst), 0);

    let term = open(
        &handle,
        carrier.clone(),
        OpenRequest {
            expected_key: Some(key_a),
            ..OpenRequest::new(80, 24)
        },
    )
    .await
    .expect("the pinned node gets a shell");
    assert_eq!(term.identity().public_key, key_a);
    drop(node_a);
}

#[tokio::test]
async fn a_terminal_open_when_its_board_is_replaced_ends_with_a_reason_and_is_not_reopened() {
    let dir_a = scratch("ended-a");
    fresh(&dir_a);
    let node_a = start_node(&dir_a, Mode::Running, None);
    let port = node_a.network().port();
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, port);
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let mut term = open(&handle, carrier.clone(), OpenRequest::new(80, 24))
        .await
        .expect("a shell on the first node");

    drop(node_a);
    let dir_b = scratch("ended-b");
    fresh(&dir_b);
    let node_b = start_node(&dir_b, Mode::Running, Some(port));

    // The watcher ends the terminal on the first state it sees that is not connected: Lost
    // while the old session is down, or Replaced once the other node answers. Either way it
    // is not reopened on the other node.
    let end = loop {
        match tokio::time::timeout(DEADLINE, term.recv())
            .await
            .expect("the terminal ends")
        {
            Some(Event::Ended(end)) => break end,
            Some(Event::Output(_)) => {}
            None => panic!("the terminal ended without a reason"),
        }
    };
    assert!(
        matches!(end, End::Lost(_) | End::Replaced(_)),
        "the terminal ends with the session's state: {end:?}"
    );
    assert_eq!(
        carrier.opens.load(Ordering::SeqCst),
        1,
        "no shell was reopened"
    );
    assert_eq!(node_b.calls_received(), 0);
    drop(node_b);
}
