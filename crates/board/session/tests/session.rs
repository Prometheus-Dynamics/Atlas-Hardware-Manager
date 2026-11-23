//! The session against the simulated node: a call runs on the live client, a call on a node
//! that is gone says so at once, a call that fails on a live session is a failure, and a node
//! turned off has no handle until it is turned on.

#![cfg(target_os = "linux")]

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

mod common;

use atlas_session::{FailureKind, Handle, Pause, Presence, RealPause, SessionError, Sessions};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use common::scratch;
use orion::{Address, ClientError};

/// A pause that does not wait: the failure rule's delays are tested on the driver side. It only
/// yields, so the monitor thread does not spin while the node is gone.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

/// A name of any length gives a node directory whose socket path fits a Unix socket, and the
/// readable name is kept in the directory. The path is checked before anything binds.
#[test]
fn a_long_name_still_gives_a_socket_that_fits() {
    let name = "session-".to_owned() + &"n".repeat(200);
    let dir = common::scratch(&name);
    let socket = common::socket_in(&dir);
    assert!(
        atlas_types::socket::check_socket_path(&socket).is_ok(),
        "{socket:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(atlas_types::socket::NAME_FILE)).expect("the name"),
        name
    );
}

fn start(dir: &Path, port: Option<u16>) -> SimNode {
    let _ = std::fs::remove_dir_all(dir);
    start_again(dir, port)
}

/// The same node in the same directory: its key is kept, so it is the same board.
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

/// Waits until the handle's monitor reports the node connected.
async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
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
        DEADLINE,
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
        matches!(&refused, Err(SessionError::Failed(reason)) if reason.text().contains("refused by the test")),
        "{refused:?}"
    );
    // The session is still up after a failed call.
    assert!(matches!(handle.presence(), Presence::Found(_)));
}

#[tokio::test]
async fn a_busy_board_is_asked_again_on_the_backoff_and_the_caller_never_sees_the_busy() {
    let dir = scratch("busy-again");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    // The board has no room twice (Orion's busy: the call was not taken), then takes it.
    let mut asked = 0_u32;
    let started = Instant::now();
    let answer = handle
        .with_client(move |_| -> Result<u32, orion::Error> {
            asked += 1;
            if asked <= 2 {
                Err(orion::Error::Client(ClientError::refused(
                    orion::types::Error::Busy,
                    "",
                )))
            } else {
                Ok(asked)
            }
        })
        .await;
    assert_eq!(answer.ok(), Some(3), "asked three times, answered once");
    // The two waits are the failure rule's first two delays (250 ms, then 500 ms).
    assert!(
        started.elapsed() >= Duration::from_millis(750),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(handle.presence(), Presence::Found(_)));

    // Any other refusal is handed back at once, asked only once.
    let mut refused_asked = 0_u32;
    let refused = handle
        .with_client(move |_| -> Result<(), orion::Error> {
            refused_asked += 1;
            assert_eq!(
                refused_asked, 1,
                "a refusal that is not busy is never asked again"
            );
            Err(orion::Error::Client(ClientError::refused(
                orion::types::Error::DigestMismatch,
                "",
            )))
        })
        .await;
    match refused {
        Err(SessionError::Failed(failure)) => assert_eq!(failure.kind(), FailureKind::Refused),
        other => panic!("expected the refusal, got {other:?}"),
    }
}

/// Orion's refusal for want of room: code busy, with no words of its own.
fn busy() -> orion::Error {
    orion::Error::Client(ClientError::refused(orion::types::Error::Busy, ""))
}

#[tokio::test]
async fn what_another_holds_is_refused_at_once_with_the_holders_words_never_waited_out() {
    let dir = scratch("busy-held");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    let mut asked = 0_u32;
    let started = Instant::now();
    let refused = handle
        .with_client(move |_| -> Result<(), orion::Error> {
            asked += 1;
            assert_eq!(asked, 1, "a held refusal is never asked again");
            Err(orion::Error::Client(ClientError::refused(
                orion::types::Error::Busy,
                "slots is held by operator:bench since 1200 ms: an install",
            )))
        })
        .await;
    match refused {
        Err(SessionError::Failed(failure)) => {
            assert_eq!(failure.kind(), FailureKind::Held);
            assert_eq!(
                failure.text(),
                "slots is held by operator:bench since 1200 ms: an install"
            );
        }
        other => panic!("expected the held refusal, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(*handle.busy_wait().borrow(), None);
}

#[tokio::test]
async fn a_board_busy_past_the_bound_is_a_refusal_with_its_words_and_the_wait_was_shown() {
    let dir = scratch("busy-bound");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle(&address(node.network().port()))
        .unwrap()
        .with_busy_bound(Duration::from_secs(6));
    connected(&handle).await;
    let mut shown = handle.busy_wait();
    assert_eq!(*shown.borrow_and_update(), None, "nothing waits yet");

    let started = Instant::now();
    let call = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .with_client(|_| -> Result<(), orion::Error> { Err(busy()) })
                .await
        })
    };
    // The wait is listed before it has lasted long: by the try whose delay crosses two seconds (the
    // fourth at the latest, which starts 1.75 s in on an unloaded machine).
    tokio::time::timeout(Duration::from_secs(5), shown.changed())
        .await
        .expect("the wait is shown in time")
        .expect("the session is up");
    let wait = shown.borrow_and_update().expect("a wait is shown");
    assert_eq!(wait.bound, Duration::from_secs(6));

    // Past the bound it is the refusal it is: Orion's busy, with the board's own words.
    let refused = call.await.expect("the call ended");
    match refused {
        Err(SessionError::Failed(failure)) => {
            assert_eq!(failure.kind(), FailureKind::Busy);
            assert_eq!(failure.text(), "busy");
        }
        other => panic!("expected the busy refusal, got {other:?}"),
    }
    // It ends when its next try would come after the bound, never later. The delays are 250, 500,
    // 1000, 2000 ms (3.75 s in all), and the 4 s one after them would pass the 6 s bound. A loaded
    // machine only makes each try later, so 3.75 s is the least it can take; the bounds on a
    // loaded machine's lateness are left wide (a 4 s bound ended early under load, 2026-11-23).
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(3750) && took < Duration::from_secs(12),
        "{took:?}"
    );
    // The call is over, so nothing is shown any more, and the session is still up.
    assert_eq!(*handle.busy_wait().borrow(), None);
    assert!(matches!(handle.presence(), Presence::Found(_)));
}

#[tokio::test]
async fn a_person_stops_a_busy_wait_and_the_call_ends_at_once_as_the_refusal() {
    let dir = scratch("busy-stop");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    let call = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .with_client(|_| -> Result<(), orion::Error> { Err(busy()) })
                .await
        })
    };
    let mut shown = handle.busy_wait();
    tokio::time::timeout(Duration::from_secs(3), shown.wait_for(Option::is_some))
        .await
        .expect("the wait is shown in time")
        .expect("the session is up");
    let stopped = Instant::now();
    handle.stop_busy();
    let refused = tokio::time::timeout(Duration::from_secs(1), call)
        .await
        .expect("the call ends when the wait is stopped, not at the 30 s bound")
        .expect("the call ended");
    assert!(
        matches!(&refused, Err(SessionError::Failed(failure)) if failure.kind() == FailureKind::Busy),
        "{refused:?}"
    );
    assert!(stopped.elapsed() < Duration::from_secs(1));
    assert_eq!(*handle.busy_wait().borrow(), None);
}

#[tokio::test]
async fn a_denied_refusal_keeps_its_kind_whatever_its_words() {
    let dir = scratch("denied-kind");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    // Reworded on purpose: the kind decides, not the text.
    let reworded = handle
        .with_client(|_| -> Result<(), orion::Error> {
            Err(orion::Error::Client(ClientError::Denied("no")))
        })
        .await;
    match reworded {
        Err(SessionError::Failed(failure)) => {
            assert_eq!(failure.kind(), FailureKind::Denied, "{failure:?}");
            assert_eq!(failure.text(), "no");
        }
        other => panic!("expected a failed call, got {other:?}"),
    }

    // The comm core's own `denied` carries its code.
    let core = handle
        .with_client(|_| -> Result<(), orion::Error> {
            Err(orion::Error::Client(ClientError::refused(
                orion::types::Error::Denied,
                "",
            )))
        })
        .await;
    match core {
        Err(SessionError::Failed(failure)) => {
            assert!(failure.is_denied(), "{failure:?}");
            assert_eq!(failure.code(), Some(orion::types::Error::Denied.code()));
        }
        other => panic!("expected a failed call, got {other:?}"),
    }
}

#[tokio::test]
async fn a_non_denied_failure_whose_text_says_denied_is_not_denied() {
    let dir = scratch("not-denied-kind");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    let answer = handle
        .with_client(|_| -> Result<(), orion::Error> {
            Err(orion::Error::Client(ClientError::Protocol("denied")))
        })
        .await;
    match answer {
        Err(SessionError::Failed(failure)) => {
            assert_eq!(failure.kind(), FailureKind::Other, "{failure:?}");
            assert!(!failure.is_denied());
            assert!(failure.text().contains("denied"));
        }
        other => panic!("expected a failed call, got {other:?}"),
    }
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

    // The same node again: the same directory keeps its key, as a board keeps its own.
    let _second = start_again(&dir, Some(port));
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
    // The release is sent whether or not anything is still waiting for it. On a loaded machine
    // the monitor may not have taken the call up within the call wait at all: the caller then gave
    // up on a call that was still queued, and a queued call whose caller gave up is dropped
    // unstarted (the next test), which drops its end of this channel. Both ways the call is gone
    // and the session must have survived, which is what this test is about.
    let _ = release.send(());
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
    let mut started = Some(started);
    let blocker = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .with_client(move |_| -> Result<(), orion::Error> {
                    if let Some(started) = started.take() {
                        let _ = started.send(());
                    }
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

#[tokio::test]
async fn a_refusal_keeps_the_nodes_words_verbatim() {
    let dir = scratch("words");
    let node = start(&dir, None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(node.network().port())).unwrap();
    connected(&handle).await;

    let detail = "lease lost while quiet: taken by atlas-next";
    let answer = handle
        .with_client(move |_| -> Result<(), orion::Error> {
            Err(orion::Error::Client(ClientError::refused(
                orion::types::Error::LeaseLost,
                detail,
            )))
        })
        .await;
    match answer {
        Err(SessionError::Failed(failure)) => {
            assert_eq!(failure.text(), detail);
            assert_eq!(failure.code(), Some(22));
        }
        other => panic!("expected a failed call, got {other:?}"),
    }
}
