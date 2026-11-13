//! The session against the simulated node: a call runs on the live client, a call on a node
//! that is gone says so at once, a call that fails on a live session is a failure, and a node
//! turned off has no handle until it is turned on.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_session::{Handle, Pause, Presence, RealPause, SessionError, Sessions};
use atlas_sim_node::{Hello, Kind, Mode, SimNode};
use orion::{Address, ClientError};

const WAIT: Duration = Duration::from_secs(15);

/// A pause that does not wait: the failure rule's delays are tested on the driver side. It only
/// yields, so the monitor thread does not spin while the node is gone.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
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

#[tokio::test]
async fn a_call_that_is_not_answered_in_the_call_wait_is_gone_and_the_session_survives() {
    let dir = scratch("timeout");
    let node = start(&dir, None);
    let wait = Duration::from_millis(100);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), wait);
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    // The call blocks the monitor's thread until the test releases it, so the caller's wait runs
    // out first. The wait is the configured one, not the five-second default.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let began = Instant::now();
    let answer = handle
        .with_client(move |_| -> Result<(), orion::Error> {
            let _ = held.recv();
            Ok(())
        })
        .await;
    let waited = began.elapsed();
    assert!(
        matches!(&answer, Err(SessionError::Gone(reason)) if reason.contains("did not answer in time")),
        "{answer:?}"
    );
    assert!(
        waited >= wait,
        "the caller waited the configured call wait: {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(4),
        "and not the five-second default: {waited:?}"
    );

    // A timed-out call does not end the session: the node is still connected once released.
    release.send(()).unwrap();
    assert!(matches!(handle.presence(), Presence::Found(_)));
}

#[tokio::test]
async fn a_queued_call_whose_caller_gave_up_is_not_started_later() {
    let dir = scratch("stale");
    let node = start(&dir, None);
    let wait = Duration::from_millis(300);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), wait);
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    // The first call holds the monitor until released; the second queues behind it and its
    // caller gives up.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (started, is_started) = tokio::sync::oneshot::channel::<()>();
    let blocker = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .with_client(move |_| -> Result<(), orion::Error> {
                    let _ = started.send(());
                    let _ = held.recv();
                    Ok(())
                })
                .await
        })
    };
    // The blocking call is on the monitor's thread before the second one is queued behind it.
    is_started.await.expect("the blocking call starts");
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let queued = handle
        .with_client(move |_| -> Result<(), orion::Error> {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
    assert!(matches!(queued, Err(SessionError::Gone(_))), "{queued:?}");
    let _ = blocker.await;

    // Released: the monitor reaches the abandoned call and must not run it. A call after it
    // answers, which shows the queue has moved on.
    release.send(()).unwrap();
    let after = handle
        .with_client(|_| -> Result<&'static str, orion::Error> { Ok("answered") })
        .await;
    assert_eq!(after.as_deref(), Ok("answered"));
    assert!(
        !ran.load(Ordering::SeqCst),
        "the abandoned call was not run"
    );
}
