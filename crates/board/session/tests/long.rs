//! Long operations on the session: a transfer advanced in steps between calls. The transfer is
//! the test double (`atlas_session::double`), so the proof is the order of events on the session
//! thread, not durations. The node is the simulated one; a loss is a node that goes away.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_session::double::{Double, Event, Log};
use atlas_session::{
    Cancel, Handle, MAX_LONG_OPERATIONS, Outcome, Pause, Presence, STEP_WAIT, SessionError,
    Sessions, Step, StepError, SteppedTransfer,
};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use orion::{Address, Orion};

/// The monitor's wait between tries: it only yields, so a node that is gone is not spun on.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("long-{name}"))
}

fn start_node(dir: &Path, port: Option<u16>) -> SimNode {
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

async fn replaced(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Replaced { .. })),
    )
    .await
    .expect("the other node is seen as replaced")
    .expect("the monitor is running");
}

/// Waits until `check` holds, polling. The deadline only ends a test whose condition never comes.
async fn until(what: &str, check: impl Fn() -> bool) {
    let end = Instant::now() + DEADLINE;
    while !check() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A transfer as the start closure of [`Handle::long`] wants it. The closure is `FnOnce`: the
/// transfer is created on the session thread by it, and only used there.
fn start(
    transfer: impl SteppedTransfer<String> + Send + 'static,
) -> impl FnOnce(&Orion) -> Result<Box<dyn SteppedTransfer<String>>, orion::Error> + Send + 'static
{
    move |_orion| Ok(Box::new(transfer) as Box<dyn SteppedTransfer<String>>)
}

/// The handle to a node, for long operations: their answer may take as long as the transfer does.
fn long_handle(handle: &Handle) -> Handle {
    handle.waiting(DEADLINE)
}

/// A call that records itself, and answers the node's name.
fn labelled_call(
    log: &Log,
    label: &str,
) -> impl FnOnce(&Orion) -> Result<String, orion::Error> + Send + 'static {
    let log = log.clone();
    let label = label.to_owned();
    move |orion| {
        log.push(Event::Call(label));
        orion.node().map(|node| node.name)
    }
}

#[tokio::test]
async fn a_short_call_completes_while_a_long_operation_is_still_running() {
    let node = start_node(&scratch("short"), None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let log = Log::new();
    let released = Arc::new(AtomicBool::new(false));
    let cancel = Cancel::new();
    let op_handle = long_handle(&handle);
    let op_log = log.clone();
    let op_flag = Arc::clone(&released);
    let op_cancel = cancel.clone();
    let op = tokio::spawn(async move {
        op_handle
            .long(
                &op_cancel,
                start(Double::until_released("push", &op_log, &op_flag)),
            )
            .await
    });
    until("the operation is stepping", || log.steps_of("push") >= 2).await;

    let name = handle
        .with_client(labelled_call(&log, "short"))
        .await
        .expect("the short call answers while the operation runs");
    assert!(!name.is_empty());
    assert_eq!(
        log.position(|event| matches!(event, Event::Done(_))),
        None,
        "the operation has not finished when the call has answered"
    );

    released.store(true, Ordering::SeqCst);
    let outcome = op
        .await
        .expect("the task ends")
        .expect("the operation ends");
    assert_eq!(outcome, Outcome::Done("push".to_owned()));
    let call_at = log
        .position(|e| matches!(e, Event::Call(_)))
        .expect("the call ran");
    let done_at = log
        .position(|e| matches!(e, Event::Done(_)))
        .expect("it finished");
    assert!(
        call_at < done_at,
        "the call ran before the operation finished"
    );
    drop(node);
}

#[tokio::test]
async fn two_long_operations_and_a_stream_of_short_calls_all_make_progress() {
    let node = start_node(&scratch("stream"), None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let log = Log::new();
    let released = Arc::new(AtomicBool::new(false));
    let mut ops = Vec::new();
    for name in ["push-a", "push-b"] {
        let op_handle = long_handle(&handle);
        let op_log = log.clone();
        let op_flag = Arc::clone(&released);
        ops.push(tokio::spawn(async move {
            op_handle
                .long(
                    &Cancel::new(),
                    start(Double::until_released(name, &op_log, &op_flag)),
                )
                .await
        }));
    }
    until("both operations are stepping", || {
        log.steps_of("push-a") >= 2 && log.steps_of("push-b") >= 2
    })
    .await;

    let (before_a, before_b) = (log.steps_of("push-a"), log.steps_of("push-b"));
    for index in 0..100 {
        let label = format!("call-{index}");
        handle
            .with_client(labelled_call(&log, &label))
            .await
            .expect("every short call answers while both operations run");
    }
    assert!(
        log.steps_of("push-a") > before_a,
        "operation A went on during the calls"
    );
    assert!(
        log.steps_of("push-b") > before_b,
        "operation B went on during the calls"
    );
    assert_eq!(
        log.position(|event| matches!(event, Event::Done(_))),
        None,
        "neither operation finished while the calls were answered"
    );

    released.store(true, Ordering::SeqCst);
    for op in ops {
        op.await
            .expect("the task ends")
            .expect("the operation ends");
    }
    let calls = log
        .events()
        .iter()
        .filter(|event| matches!(event, Event::Call(_)))
        .count();
    assert_eq!(calls, 100, "every call ran once");
    // Interleaved, not one after the other: B takes a step before A has taken its last.
    let last_a = log
        .events()
        .iter()
        .rposition(|e| matches!(e, Event::Step { name, .. } if name == "push-a"))
        .unwrap();
    let first_b = log
        .position(|e| matches!(e, Event::Step { name, .. } if name == "push-b"))
        .unwrap();
    assert!(first_b < last_a, "the two operations were stepped in turns");
    drop(node);
}

#[tokio::test]
async fn a_cancel_lands_at_the_next_step_boundary() {
    let node = start_node(&scratch("cancel"), None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let log = Log::new();
    let cancel = Cancel::new();
    // The hook runs on the session thread right after step three, and asks for the cancel there.
    let asker = cancel.clone();
    let transfer = Double::steps("copy", &log, 100).after_each_step(move |n| {
        if n == 3 {
            asker.cancel();
        }
    });
    let outcome = handle.long(&cancel, start(transfer)).await;
    assert_eq!(outcome, Ok(Outcome::Cancelled));
    assert_eq!(log.steps_of("copy"), 3, "no step began after the cancel");
    assert_eq!(
        log.events()
            .iter()
            .filter(|e| matches!(e, Event::Cancelled(_)))
            .count(),
        1,
        "the transfer was told to stop once"
    );
    assert_eq!(log.position(|e| matches!(e, Event::Done(_))), None);
    drop(node);
}

#[tokio::test]
async fn a_lost_session_ends_the_transfer_and_queued_calls_and_nothing_steps_after() {
    let dir = scratch("lost");
    let node = start_node(&dir, None);
    let port = node.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let log = Log::new();
    let released = Arc::new(AtomicBool::new(false));
    let op_handle = long_handle(&handle);
    let op_log = log.clone();
    let op_flag = Arc::clone(&released);
    let op = tokio::spawn(async move {
        op_handle
            .long(
                &Cancel::new(),
                start(Double::until_released("push", &op_log, &op_flag)),
            )
            .await
    });
    until("the operation is stepping", || log.steps_of("push") >= 2).await;

    // Calls issued as the node goes away: each is answered as gone, never run on another session.
    let mut calls = Vec::new();
    for index in 0..3 {
        let call_handle = handle.clone();
        let call_log = log.clone();
        calls.push(tokio::spawn(async move {
            call_handle
                .with_client(labelled_call(&call_log, &format!("queued-{index}")))
                .await
        }));
    }
    drop(node);
    let outcome = op.await.expect("the task ends");
    assert!(
        matches!(outcome, Err(SessionError::Gone(_))),
        "the transfer ends as a lost call does: {outcome:?}"
    );
    for call in calls {
        let answer = call.await.expect("the call task ends");
        if let Ok(name) = &answer {
            // A call that ran before the loss was noticed ran on the live session: that is allowed.
            assert!(!name.is_empty());
        } else {
            assert!(matches!(answer, Err(SessionError::Gone(_))), "{answer:?}");
        }
    }
    lost(&handle).await;

    // The node comes back. The ended transfer is not resumed by the session: its steps stay put.
    let steps_at_end = log.steps_of("push");
    let node = start_node(&dir, Some(port));
    connected(&handle).await;
    handle
        .with_client(labelled_call(&log, "after"))
        .await
        .expect("calls work again on the new session");
    assert_eq!(
        log.steps_of("push"),
        steps_at_end,
        "nothing stepped the ended transfer"
    );
    drop(node);
}

#[tokio::test]
async fn a_replaced_board_gets_no_step_and_no_long_operation_starts_on_it() {
    let dir_a = scratch("replace-a");
    let node_a = start_node(&dir_a, None);
    let port = node_a.network().port();
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let log = Log::new();
    let released = Arc::new(AtomicBool::new(false));
    let op_handle = long_handle(&handle);
    let op_log = log.clone();
    let op_flag = Arc::clone(&released);
    let op = tokio::spawn(async move {
        op_handle
            .long(
                &Cancel::new(),
                start(Double::until_released("push", &op_log, &op_flag)),
            )
            .await
    });
    until("the operation is stepping", || log.steps_of("push") >= 1).await;
    drop(node_a);
    let outcome = op.await.expect("the task ends");
    assert!(matches!(outcome, Err(SessionError::Gone(_))), "{outcome:?}");
    let steps_at_end = log.steps_of("push");

    // Another node answers at the address: it is replaced, and nothing is started on it.
    let node_b = start_node(&scratch("replace-b"), Some(port));
    replaced(&handle).await;
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    let late_log = log.clone();
    let late = handle
        .long(&Cancel::new(), move |orion| {
            flag.store(true, Ordering::SeqCst);
            let _ = orion;
            Ok(Box::new(Double::steps("late", &late_log, 1)) as Box<dyn SteppedTransfer<String>>)
        })
        .await;
    assert!(matches!(late, Err(SessionError::Gone(_))), "{late:?}");
    assert!(
        !started.load(Ordering::SeqCst),
        "no start ran on the other node"
    );
    assert_eq!(log.steps_of("late"), 0);
    assert_eq!(
        log.steps_of("push"),
        steps_at_end,
        "the first node's transfer took no more steps"
    );
    assert_eq!(
        node_b.calls_received(),
        0,
        "the other node received no call"
    );
    drop(node_b);
}

#[tokio::test]
async fn a_transfer_that_fails_for_good_answers_failed_and_the_session_carries_on() {
    let node = start_node(&scratch("failed"), None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    struct Refused;
    impl SteppedTransfer<String> for Refused {
        fn step(&mut self, _wait: Duration) -> Result<Step<String>, StepError> {
            Err(StepError::Failed("the board refused the file".to_owned()))
        }
        fn cancel(&mut self) {}
    }
    let outcome = handle.long(&Cancel::new(), start(Refused)).await;
    assert_eq!(
        outcome,
        Err(SessionError::Failed(
            "the board refused the file".to_owned()
        ))
    );
    let name = handle
        .with_client(|orion| orion.node().map(|node| node.name))
        .await
        .expect("the session still answers calls");
    assert!(!name.is_empty());
    drop(node);
}

#[tokio::test]
async fn no_more_than_the_limit_of_long_operations_step_at_once() {
    let node = start_node(&scratch("limit"), None);
    let sessions = Sessions::new(Arc::new(NoPause));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    assert_eq!(MAX_LONG_OPERATIONS, 4, "the limit this test is written for");
    assert_eq!(
        STEP_WAIT,
        Duration::from_millis(50),
        "the step wait this test is written for"
    );

    let log = Log::new();
    let released = Arc::new(AtomicBool::new(false));
    let mut ops = Vec::new();
    let names = ["one", "two", "three", "four", "five"];
    for name in names {
        let op_handle = long_handle(&handle);
        let op_log = log.clone();
        let op_flag = Arc::clone(&released);
        ops.push(tokio::spawn(async move {
            op_handle
                .long(
                    &Cancel::new(),
                    start(Double::until_released(name, &op_log, &op_flag)),
                )
                .await
        }));
        if name == "four" {
            until("four operations are stepping", || {
                ["one", "two", "three", "four"]
                    .iter()
                    .all(|n| log.steps_of(n) >= 2)
            })
            .await;
        }
    }
    until("the fifth is waiting for a slot", || {
        log.steps_of("one") >= 4
    })
    .await;
    assert_eq!(log.steps_of("five"), 0, "the fifth is parked, not stepped");

    released.store(true, Ordering::SeqCst);
    for op in ops {
        op.await
            .expect("the task ends")
            .expect("the operation ends");
    }
    let first_done = log
        .position(|e| matches!(e, Event::Done(_)))
        .expect("one finished");
    let fifth_first = log
        .position(|e| matches!(e, Event::Step { name, .. } if name == "five"))
        .expect("the fifth ran once a slot freed");
    assert!(
        first_done < fifth_first,
        "the fifth started only after a slot freed"
    );
    drop(node);
}
