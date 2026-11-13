//! The lease: a session keeps a board's lease alive while it holds something on the board, and sends
//! nothing for it otherwise. The node is the simulated one, which counts the requests it receives
//! per action name. Lease periods are short for these tests (a renew every 300 ms or 2 s); no test
//! waits for a real minute. Every wait has a generous deadline: a timing failure is fixed, not rerun.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_session::double::{Double, Event, Log};
use atlas_session::{Cancel, Handle, Lease, Pause, Presence, Sessions, SteppedTransfer};
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
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lease-{name}"))
}

fn with_lemnos() -> Kind {
    Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    })
}

fn start_node(dir: &Path, kind: &Kind, port: Option<u16>) -> SimNode {
    SimNode::start_with(dir, kind, port).expect("the simulated node starts")
}

fn address(port: u16) -> Address {
    Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap()
}

/// The bridge requests the node took: a device's `.call`, which is what a renewal is.
fn renewals(node: &SimNode) -> usize {
    node.bridge_calls_started()
}

/// A lease that renews every `renew` and is never lost to silence in these tests.
fn lease(renew: Duration) -> Lease {
    Lease {
        period: Duration::from_secs(30),
        renew,
    }
}

fn sessions(lease: Lease) -> Sessions {
    Sessions::new(Arc::new(NoPause)).with_lease(lease)
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
    .expect("the session notices the node is gone")
    .expect("the monitor is running");
}

async fn replaced(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Replaced { .. })),
    )
    .await
    .expect("the session sees another node at the address")
    .expect("the monitor is running");
}

async fn until(what: &str, check: impl Fn() -> bool) {
    let end = Instant::now() + DEADLINE;
    while !check() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A bridge request from the holder: a call that reaches the bridge, so it renews the lease.
async fn bridge_read(handle: &Handle) {
    let _ = handle
        .with_bridge(|orion: &Orion| orion.node().map(|node| node.name))
        .await;
}

#[tokio::test]
async fn with_no_hold_the_session_sends_no_renewal() {
    let node = start_node(&scratch("no-hold"), &with_lemnos(), None);
    let sessions = sessions(lease(Duration::from_millis(100)));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(renewals(&node), 0, "no hold, so no renewal");
    assert_eq!(node.calls_received(), 0, "no hold, so no request at all");
}

#[tokio::test]
async fn a_hold_with_no_other_traffic_is_renewed_within_the_period() {
    let node = start_node(&scratch("renewed"), &with_lemnos(), None);
    let sessions = sessions(lease(Duration::from_millis(300)));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    until("the first renewal", || renewals(&node) >= 1).await;
    until("a second renewal", || renewals(&node) >= 2).await;
}

#[tokio::test]
async fn the_holders_own_bridge_traffic_suppresses_renewals() {
    let node = start_node(&scratch("traffic"), &with_lemnos(), None);
    // A renewal is due after 2 s of silence. The holder makes a bridge request every 100 ms, so
    // none is due for the whole run.
    let sessions = sessions(lease(Duration::from_secs(2)));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        bridge_read(&handle).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        renewals(&node),
        0,
        "the holder's own requests renewed the lease, so none was sent"
    );
}

#[tokio::test]
async fn two_holders_keep_the_lease_until_both_are_gone() {
    let node = start_node(&scratch("two"), &with_lemnos(), None);
    let sessions = sessions(lease(Duration::from_millis(300)));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let first = handle.hold();
    let second = handle.hold();
    until("a renewal with both holds", || renewals(&node) >= 1).await;

    drop(first);
    let before = renewals(&node);
    until("a renewal with one hold left", || renewals(&node) > before).await;

    drop(second);
    // Renewals may already be in flight when the last hold goes; they settle within a period.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let settled = renewals(&node);
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(
        renewals(&node),
        settled,
        "no hold is left, so no renewal is sent"
    );
}

#[tokio::test]
async fn a_lost_session_stops_renewals_and_they_resume_on_the_same_node() {
    let dir = scratch("lost");
    let kind = with_lemnos();
    let node = start_node(&dir, &kind, None);
    let port = node.network().port();
    let sessions = sessions(lease(Duration::from_millis(300)));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    until("a renewal before the loss", || renewals(&node) >= 1).await;
    let key = handle.node_key().expect("connected before the loss");

    drop(node);
    lost(&handle).await;
    let back = start_node(&dir, &kind, Some(port));
    connected(&handle).await;
    until("a renewal once the same node is back", || {
        renewals(&back) >= 1
    })
    .await;
    assert_eq!(handle.node_key(), Some(key), "the same node is back");
}

#[tokio::test]
async fn a_replaced_board_gets_no_renewal() {
    let dir_a = scratch("replaced-a");
    let kind = with_lemnos();
    let node_a = start_node(&dir_a, &kind, None);
    let port = node_a.network().port();
    let sessions = sessions(lease(Duration::from_millis(300)));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    drop(node_a);
    lost(&handle).await;

    let node_b = start_node(&scratch("replaced-b"), &kind, Some(port));
    replaced(&handle).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(renewals(&node_b), 0, "the other board is sent no renewal");
    assert_eq!(
        node_b.calls_received(),
        0,
        "the other board is sent no call"
    );
}

#[tokio::test]
async fn a_node_without_lemnos_has_no_lease_and_gets_nothing() {
    let node = start_node(&scratch("plain"), &Kind::Plain, None);
    let sessions = sessions(lease(Duration::from_millis(100)));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(
        node.calls_received(),
        0,
        "a plain Orion node has no bridge: nothing is sent for the hold"
    );
}

#[tokio::test]
async fn a_renewal_is_not_starved_by_a_long_operation() {
    let node = start_node(&scratch("long"), &with_lemnos(), None);
    let sessions = sessions(lease(Duration::from_millis(300)));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    let log = Log::new();
    let released = Arc::new(AtomicBool::new(false));
    let cancel = Cancel::new();
    let transfer = Double::until_released("push", &log, &released);
    let operation = {
        let handle = handle.waiting(DEADLINE);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            handle
                .long(&cancel, move |_orion: &Orion| {
                    Ok(Box::new(transfer) as Box<dyn SteppedTransfer<String>>)
                })
                .await
        })
    };
    // The operation is running (it steps until released) while renewals keep going.
    until("two renewals while the operation runs", || {
        renewals(&node) >= 2
    })
    .await;
    assert!(!operation.is_finished(), "the operation is still running");
    assert!(
        log.events()
            .iter()
            .any(|event| matches!(event, Event::Step { .. })),
        "the operation stepped while the renewals went out"
    );
    released.store(true, Ordering::SeqCst);
    operation
        .await
        .expect("the task ends")
        .expect("the operation ends");
}

#[tokio::test]
async fn a_silent_session_with_a_hold_records_a_lapse_and_one_without_does_not() {
    let node = start_node(&scratch("lapse"), &with_lemnos(), None);
    // Renewals never come (a long renew), and the period is short: silence is a lapse at once.
    let sessions = sessions(Lease {
        period: Duration::from_millis(300),
        renew: Duration::from_secs(3600),
    });
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // No hold: silence is not a lapse.
    let mut lapses = handle.lapses();
    let quiet = tokio::time::timeout(Duration::from_millis(900), lapses.changed()).await;
    assert!(quiet.is_err(), "no hold, so no lapse");

    let mut hold = handle.hold();
    tokio::time::timeout(DEADLINE, lapses.changed())
        .await
        .expect("the silence with a hold is a lapse")
        .expect("the monitor is running");
    assert!(hold.lapsed(), "the hold sees the lapse");
    hold.acknowledge();
    assert!(!hold.lapsed(), "an acknowledged lapse is not seen again");
    assert_eq!(renewals(&node), 0, "the renew period never came");
}

#[tokio::test]
async fn holds_are_per_board() {
    let node_a = start_node(&scratch("per-board-a"), &with_lemnos(), None);
    let node_b = start_node(&scratch("per-board-b"), &with_lemnos(), None);
    let sessions = sessions(lease(Duration::from_millis(300)));
    let a = sessions
        .handle(&address(node_a.network().port()))
        .expect("the node is on");
    let b = sessions
        .handle(&address(node_b.network().port()))
        .expect("the node is on");
    connected(&a).await;
    connected(&b).await;

    let _hold = a.hold();
    until("a renewal on the held board", || renewals(&node_a) >= 1).await;
    assert_eq!(
        renewals(&node_b),
        0,
        "a hold on one board sends nothing to another"
    );
    assert_eq!(node_b.calls_received(), 0);
}
