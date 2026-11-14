//! The key a session saw at its address is its pin. A node back under the same address with the
//! same key is found again; another key is reported as a replacement, is sent no call, and is
//! accepted only by a person's act, which starts a fresh session. A session opened with a known
//! key refuses another node on its very first connect.
//!
//! The simulated node counts the calls it is asked to start, so "the node saw no call" is read on
//! its side, not inferred. Every wait ends at a generous deadline; nothing is asserted by sleeping.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use atlas_session::{Handle, Pause, Presence, SessionError, Sessions};
use atlas_sim_node::names::SLOTS_LIST;
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode, call};
use orion::{Address, Identity, Options, Orion};

/// A pause that does not wait: the failure rule's delays are tested on the driver side.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pin-{name}"))
}

/// A simulated node in `dir` (a fresh one: its key is new), on `port` if given.
fn start(dir: &Path, port: Option<u16>) -> SimNode {
    let _ = std::fs::remove_dir_all(dir);
    start_again(dir, port)
}

/// The same node again: the same directory means the same key.
fn start_again(dir: &Path, port: Option<u16>) -> SimNode {
    let kind = Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    });
    SimNode::start_with(dir, &kind, port).expect("the simulated node starts")
}

fn address(port: u16) -> Address {
    Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap()
}

/// The key a node at `port` shows, as a person's tool would learn it: one unpinned signed connect.
fn learn_key(port: u16) -> [u8; 32] {
    let options = Options {
        identity: Some(Identity::generate()),
        ..Options::default()
    };
    let orion = Orion::connect_with(&address(port), "pin-test", options)
        .expect("the node answers a signed connect");
    orion.node().expect("the hello").public_key
}

async fn wait_for(handle: &Handle, want: impl Fn(&Presence) -> bool) -> Presence {
    let mut presence = handle.watch();
    tokio::time::timeout(DEADLINE, presence.wait_for(|p| want(p)))
        .await
        .expect("the presence changes to what the test waits for")
        .expect("the monitor is running")
        .clone()
}

async fn connected(handle: &Handle) -> [u8; 32] {
    match wait_for(handle, |p| matches!(p, Presence::Found(_))).await {
        Presence::Found(node) => node.public_key,
        other => panic!("expected Found, got {other:?}"),
    }
}

async fn lost(handle: &Handle) {
    wait_for(handle, |p| matches!(p, Presence::Lost(_))).await;
}

async fn replaced(handle: &Handle) -> ([u8; 32], Option<[u8; 32]>) {
    match wait_for(handle, |p| matches!(p, Presence::Replaced { .. })).await {
        Presence::Replaced { expected, found } => (expected, found),
        other => panic!("expected Replaced, got {other:?}"),
    }
}

#[tokio::test]
async fn the_same_node_back_is_found_again_and_resumes() {
    let dir = scratch("same");
    let node = start(&dir, None);
    let port = node.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    let key = connected(&handle).await;
    assert_eq!(handle.node_key(), Some(key));
    drop(node);
    lost(&handle).await;

    let node = start_again(&dir, Some(port));
    assert_eq!(connected(&handle).await, key, "the same key comes back");
    let name = handle
        .with_client(|orion| orion.node().map(|n| n.name))
        .await
        .expect("a call runs on the found node again");
    assert!(!name.is_empty());
    drop(node);
}

#[tokio::test]
async fn a_different_key_under_the_same_address_is_replaced_and_gets_no_call() {
    let dir_a = scratch("replace-a");
    let node_a = start(&dir_a, None);
    let port = node_a.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    let key_a = connected(&handle).await;
    drop(node_a);
    lost(&handle).await;

    let dir_b = scratch("replace-b");
    let node_b = start(&dir_b, Some(port));
    let (expected, _found) = replaced(&handle).await;
    assert_eq!(
        expected, key_a,
        "the session stays pinned to the key it saw"
    );
    assert_ne!(
        learn_key(port),
        key_a,
        "the node now at the address is another node"
    );
    assert_eq!(handle.node_key(), None, "no node is found under the pin");

    // A call is refused on this side, and the other node sees none.
    let refused = handle
        .with_client(|orion| call(orion, SLOTS_LIST, &[]).map(|_| ()))
        .await;
    assert!(matches!(refused, Err(SessionError::Gone(_))), "{refused:?}");
    assert_eq!(
        node_b.calls_received(),
        0,
        "the other node received no call"
    );
    drop(node_b);
}

#[tokio::test]
async fn a_call_queued_while_the_node_is_replaced_is_not_delivered_to_the_other_node() {
    let dir_a = scratch("queued-a");
    let node_a = start(&dir_a, None);
    let port = node_a.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    // A slow call holds the monitor; a second call waits behind it. The node goes while both
    // are in hand, and another node comes back on the address.
    let slow = handle.clone();
    let slow_call = tokio::spawn(async move {
        slow.with_client(|orion| {
            std::thread::sleep(Duration::from_millis(300));
            call(orion, SLOTS_LIST, &[]).map(|_| ())
        })
        .await
    });
    let queued = handle.clone();
    let queued_call = tokio::spawn(async move {
        queued
            .with_client(|orion| call(orion, SLOTS_LIST, &[]).map(|_| ()))
            .await
    });
    drop(node_a);
    let _ = slow_call.await;
    let queued_answer = queued_call.await.expect("the call task finishes");
    assert!(
        matches!(queued_answer, Err(SessionError::Gone(_))),
        "a queued call on a gone node is not delivered: {queued_answer:?}"
    );

    let node_b = start(&scratch("queued-b"), Some(port));
    replaced(&handle).await;
    assert_eq!(
        node_b.calls_received(),
        0,
        "the other node received no queued call"
    );
    drop(node_b);
}

#[tokio::test]
async fn accepting_the_new_key_starts_a_fresh_session_and_does_not_resume_old_work() {
    let dir_a = scratch("accept-a");
    let node_a = start(&dir_a, None);
    let port = node_a.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    let key_a = connected(&handle).await;
    // A job started on the old node records the key it started on.
    let started_on = handle.node_key();
    drop(node_a);
    lost(&handle).await;

    let node_b = start(&scratch("accept-b"), Some(port));
    replaced(&handle).await;
    let key_b = learn_key(port);
    assert_ne!(key_a, key_b);

    // Nothing accepted yet: the old pin holds.
    assert_eq!(node_b.calls_received(), 0);
    // A person says this address now means key B.
    assert!(sessions.accept_key(&address(port).to_string(), key_b));
    assert_eq!(
        connected(&handle).await,
        key_b,
        "the new node is found under its own key"
    );
    assert_eq!(handle.node_key(), Some(key_b));
    assert_ne!(
        started_on,
        handle.node_key(),
        "a job from the old node does not match the new one"
    );
    let answer = handle
        .with_client(|orion| orion.node().map(|n| n.public_key))
        .await
        .expect("a call runs on the accepted node");
    assert_eq!(answer, key_b);
    drop(node_b);
}

#[tokio::test]
async fn a_session_opened_with_a_known_key_refuses_a_different_node_on_its_first_connect() {
    // Learn key A from a node that is then gone; another node B answers on the address.
    let node_a = start(&scratch("known-a"), None);
    let key_a = learn_key(node_a.network().port());
    drop(node_a);
    let node_b = start(&scratch("known-b"), None);
    let port = node_b.network().port();
    let key_b = learn_key(port);

    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle_pinned(&address(port), key_a)
        .expect("the node is on");
    let mut presence = handle.watch();
    // The first state that is not "searching" is the refusal, not a found or lost node.
    let first = tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|p| !matches!(p, Presence::Searching)),
    )
    .await
    .expect("the first connect is answered")
    .expect("the monitor is running")
    .clone();
    assert_eq!(
        first,
        Presence::Replaced {
            expected: key_a,
            found: Some(key_b)
        },
        "a different node is refused on the first connect, and its key is shown"
    );
    assert_eq!(node_b.calls_received(), 0);
    drop(node_b);
}

#[tokio::test]
async fn the_expected_node_back_after_an_impostor_is_found_again_without_anyone_acting() {
    let dir_a = scratch("back-a");
    let node_a = start(&dir_a, None);
    let port = node_a.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    let key_a = connected(&handle).await;
    drop(node_a);
    lost(&handle).await;

    let node_b = start(&scratch("back-b"), Some(port));
    replaced(&handle).await;
    drop(node_b);

    let node_a = start_again(&dir_a, Some(port));
    assert_eq!(
        connected(&handle).await,
        key_a,
        "the expected node is found again"
    );
    drop(node_a);
}
