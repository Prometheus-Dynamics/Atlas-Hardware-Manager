//! The session against the simulated node: a call runs on the live client, a call on a node
//! that is gone says so at once, a call that fails on a live session is a failure, and a node
//! turned off has no handle until it is turned on.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use atlas_session::{Handle, Pause, Presence, RealPause, SessionError, Sessions};
use atlas_sim_node::{Hello, Kind, Mode, SimNode};
use orion::{Address, ClientError};

const WAIT: Duration = Duration::from_secs(15);

/// A pause that does not sleep: the failure rule is tested on the driver side.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("session-{name}"))
}

fn start(dir: &Path, port: Option<u16>) -> SimNode {
    let _ = std::fs::remove_dir_all(dir);
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

/// Waits until the handle's monitor reports the node connected.
async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        WAIT,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected")
    .expect("the monitor is running");
}

/// Waits until the handle's monitor reports the node lost.
async fn lost(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        WAIT,
        presence.wait_for(|presence| matches!(presence, Presence::Lost(_))),
    )
    .await
    .expect("the node is reported lost")
    .expect("the monitor is running");
}

#[tokio::test]
async fn a_call_runs_on_the_live_client_and_returns_what_it_answers() {
    let dir = scratch("live");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    let name = handle
        .with_client(|orion| -> Result<String, orion::Error> { Ok(orion.node()?.name) })
        .await
        .expect("the call is answered");
    assert!(!name.is_empty());
}

#[tokio::test]
async fn a_call_that_fails_on_a_live_session_is_a_failure_not_a_gone_node() {
    let dir = scratch("failed");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    let refused = handle
        .with_client(|_| -> Result<(), orion::Error> {
            Err(orion::Error::Client(ClientError::Denied(
                "refused by the test",
            )))
        })
        .await;
    assert!(
        matches!(&refused, Err(SessionError::Failed(reason)) if reason.contains("refused by the test")),
        "{refused:?}"
    );
    // The session is still up after a failed call.
    assert!(matches!(handle.presence(), Presence::Found(_)));
}

#[tokio::test]
async fn a_call_on_a_node_that_is_gone_says_so_at_once() {
    let dir = scratch("gone");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    drop(node);
    lost(&handle).await;

    let began = Instant::now();
    let answer = handle
        .with_client(|orion| -> Result<String, orion::Error> { Ok(orion.node()?.name) })
        .await;
    assert!(
        matches!(answer, Err(SessionError::Gone(_))),
        "a call on a gone node is Gone: {answer:?}"
    );
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "a gone node answers at once, it does not wait for the call wait"
    );
}

#[tokio::test]
async fn a_node_is_found_again_on_the_same_port_and_calls_run_again() {
    let dir = scratch("again");
    let first = start(&dir, None);
    let port = first.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).unwrap();
    connected(&handle).await;
    drop(first);
    lost(&handle).await;

    let _second = start(&dir, Some(port));
    connected(&handle).await;
    let name = handle
        .with_client(|orion| -> Result<String, orion::Error> { Ok(orion.node()?.name) })
        .await;
    assert!(name.is_ok(), "{name:?}");
}

#[tokio::test]
async fn a_turned_off_node_has_no_handle_until_it_is_turned_on() {
    let sessions = Sessions::new(Arc::new(RealPause));
    let target = address(9);
    let text = target.to_string();
    assert!(sessions.existing(&text).is_none());

    sessions.turn_off(&text);
    assert!(sessions.handle(&target).is_none(), "off: no handle");

    sessions.turn_on(&text);
    let handle = sessions.handle(&target).expect("on: a handle");
    assert_eq!(handle.address(), &target);
    assert!(sessions.existing(&text).is_some());
}
