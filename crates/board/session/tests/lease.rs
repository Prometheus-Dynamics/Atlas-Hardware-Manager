//! The lease: a session keeps a board's lease alive while it holds something on the board, and sends
//! nothing for it otherwise. The node is the simulated one, which counts the requests it receives.
//!
//! Time is the test's, not the machine's: the session reads its lease on a manual clock
//! (`atlas_session::Clock::manual`), which moves only when a test advances it. So "the renew period has
//! passed" is a line in the test, and a slow or loaded machine cannot make a renewal or a lapse come
//! early or late. The waits that remain are negative checks ("nothing is sent"), which a stall can only
//! make pass more easily, and waits for an event the test has already caused, with the generous
//! `DEADLINE`.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_session::double::{Double, Event, Log};
use atlas_session::{Cancel, Clock, Handle, Lease, Pause, Presence, Sessions, SteppedTransfer};
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

/// The lease the tests use: in force, a renewal due after `RENEW` of clock time, a lapse after
/// `PERIOD` of clock time. Only the clock moves them.
const RENEW: Duration = Duration::from_secs(10);
const PERIOD: Duration = Duration::from_secs(60);

fn lease() -> Lease {
    Lease {
        period: PERIOD,
        renew: RENEW,
        in_force: true,
    }
}

fn sessions(lease: Lease, clock: &Clock) -> Sessions {
    Sessions::new(Arc::new(NoPause))
        .with_lease(lease)
        .with_clock(clock.clone())
}

/// Lets the session's own turns run a few times. Only for negative checks: nothing that is checked
/// after it can have been caused by the wait.
async fn let_turns_run() {
    tokio::time::sleep(Duration::from_millis(300)).await;
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
    let end = std::time::Instant::now() + DEADLINE;
    while !check() {
        assert!(
            std::time::Instant::now() < end,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits until the session has finished the turn it is in. A renewal counts on the node when it starts,
/// but the session stamps it when it finishes, on the clock. A call queued now runs only after that, so
/// a test that advances the clock once this returns does not race the stamp.
async fn settled(handle: &Handle) {
    let _ = handle
        .with_client(|_orion: &Orion| Ok::<(), orion::Error>(()))
        .await;
}

/// Waits for the renewals on `node` to reach `at_least`, and for the session to have settled them.
async fn renewed(handle: &Handle, node: &SimNode, at_least: usize, what: &str) {
    until(what, || renewals(node) >= at_least).await;
    settled(handle).await;
}

/// A bridge request from the holder: a call that reaches the bridge, so it renews the lease.
async fn bridge_read(handle: &Handle) {
    let _ = handle
        .with_bridge(|orion: &Orion| orion.node().map(|node| node.name))
        .await;
}

#[tokio::test]
async fn with_no_hold_the_session_sends_no_renewal() {
    let clock = Clock::manual();
    let node = start_node(&scratch("no-hold"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    clock.advance(Duration::from_secs(3600));
    let_turns_run().await;
    assert_eq!(renewals(&node), 0, "no hold, so no renewal");
    assert_eq!(node.calls_received(), 0, "no hold, so no request at all");
}

#[tokio::test]
async fn a_hold_is_renewed_when_the_renew_period_has_passed_on_the_clock() {
    let clock = Clock::manual();
    let node = start_node(&scratch("renewed"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    let_turns_run().await;
    assert_eq!(
        renewals(&node),
        0,
        "the clock has not moved: nothing is due"
    );

    clock.advance(RENEW);
    renewed(&handle, &node, 1, "the first renewal").await;
    clock.advance(RENEW);
    renewed(&handle, &node, 2, "a second renewal").await;
}

#[tokio::test]
async fn the_holders_own_bridge_traffic_suppresses_renewals() {
    let clock = Clock::manual();
    let node = start_node(&scratch("traffic"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // Each step moves the clock by half the renew period, then the holder makes a bridge request.
    // The silence never reaches the renew period, however the machine is scheduled.
    let _hold = handle.hold();
    for _ in 0..20 {
        clock.advance(RENEW / 2);
        bridge_read(&handle).await;
    }
    let_turns_run().await;
    assert_eq!(
        renewals(&node),
        0,
        "the holder's own requests renewed the lease, so none was sent"
    );
}

#[tokio::test]
async fn two_holders_keep_the_lease_until_both_are_gone() {
    let clock = Clock::manual();
    let node = start_node(&scratch("two"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let first = handle.hold();
    let second = handle.hold();
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "a renewal with both holds").await;

    drop(first);
    let before = renewals(&node);
    clock.advance(RENEW);
    until("a renewal with one hold left", || renewals(&node) > before).await;
    settled(&handle).await;

    drop(second);
    let_turns_run().await;
    let settled = renewals(&node);
    clock.advance(RENEW * 3);
    let_turns_run().await;
    assert_eq!(
        renewals(&node),
        settled,
        "no hold is left, so no renewal is sent"
    );
}

#[tokio::test]
async fn a_lost_session_stops_renewals_and_they_resume_on_the_same_node() {
    let clock = Clock::manual();
    let dir = scratch("lost");
    let kind = with_lemnos();
    let node = start_node(&dir, &kind, None);
    let port = node.network().port();
    let sessions = sessions(lease(), &clock);
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "a renewal before the loss").await;
    let key = handle.node_key().expect("connected before the loss");

    drop(node);
    lost(&handle).await;
    let back = start_node(&dir, &kind, Some(port));
    connected(&handle).await;
    assert_eq!(handle.node_key(), Some(key), "the same node is back");

    // The silence (the node was down) is well short of the lease period: no lapse, and the renewal
    // is due again as soon as the renew period has passed on the clock.
    clock.advance(RENEW);
    renewed(&handle, &back, 1, "a renewal once the same node is back").await;
}

#[tokio::test]
async fn a_replaced_board_gets_no_renewal() {
    let clock = Clock::manual();
    let dir_a = scratch("replaced-a");
    let kind = with_lemnos();
    let node_a = start_node(&dir_a, &kind, None);
    let port = node_a.network().port();
    let sessions = sessions(lease(), &clock);
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    drop(node_a);
    lost(&handle).await;

    let node_b = start_node(&scratch("replaced-b"), &kind, Some(port));
    replaced(&handle).await;
    clock.advance(RENEW * 100);
    let_turns_run().await;
    assert_eq!(renewals(&node_b), 0, "the other board is sent no renewal");
    assert_eq!(
        node_b.calls_received(),
        0,
        "the other board is sent no call"
    );
}

#[tokio::test]
async fn a_node_without_lemnos_has_no_lease_and_gets_nothing() {
    let clock = Clock::manual();
    let node = start_node(&scratch("plain"), &Kind::Plain, None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    clock.advance(RENEW * 100);
    let_turns_run().await;
    assert_eq!(
        node.calls_received(),
        0,
        "a plain Orion node has no bridge: nothing is sent for the hold"
    );
}

#[tokio::test]
async fn a_renewal_is_not_starved_by_a_long_operation() {
    let clock = Clock::manual();
    let node = start_node(&scratch("long"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
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
    // The operation steps until it is released. Each renewal is due by the clock, and the renewals
    // keep going while it runs.
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "a renewal while the operation runs").await;
    clock.advance(RENEW);
    renewed(
        &handle,
        &node,
        2,
        "a second renewal while the operation runs",
    )
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
    let clock = Clock::manual();
    let node = start_node(&scratch("lapse"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // No hold: a long silence on the clock is not a lapse.
    let mut lapses = handle.lapses();
    clock.advance(PERIOD * 10);
    let_turns_run().await;
    assert!(
        lapses.has_changed().map(|changed| !changed).unwrap_or(true),
        "no hold, so no lapse"
    );

    // A hold taken now starts its silence now: past the period on the clock, it is a lapse.
    let mut hold = handle.hold();
    let_turns_run().await;
    clock.advance(PERIOD + Duration::from_secs(1));
    tokio::time::timeout(DEADLINE, lapses.changed())
        .await
        .expect("the silence with a hold is a lapse")
        .expect("the monitor is running");
    assert!(hold.lapsed(), "the hold sees the lapse");
    hold.acknowledge();
    assert!(!hold.lapsed(), "an acknowledged lapse is not seen again");
}

#[tokio::test]
async fn holds_are_per_board() {
    let clock = Clock::manual();
    let node_a = start_node(&scratch("per-board-a"), &with_lemnos(), None);
    let node_b = start_node(&scratch("per-board-b"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let a = sessions
        .handle(&address(node_a.network().port()))
        .expect("the node is on");
    let b = sessions
        .handle(&address(node_b.network().port()))
        .expect("the node is on");
    connected(&a).await;
    connected(&b).await;

    let _hold = a.hold();
    clock.advance(RENEW);
    renewed(&a, &node_a, 1, "a renewal on the held board").await;
    let_turns_run().await;
    assert_eq!(
        renewals(&node_b),
        0,
        "a hold on one board sends nothing to another"
    );
    assert_eq!(node_b.calls_received(), 0);
}

/// At the locked revision the lease is not in force. A session with holds then sends nothing to the
/// bridge for the lease, lists no device for it, and records no lapse, however long the clock runs:
/// so no holder can be told its hold was dropped by a lease that does not exist.
#[tokio::test]
async fn with_the_lease_not_in_force_a_hold_sends_nothing_and_records_no_lapse() {
    let clock = Clock::manual();
    let node = start_node(&scratch("off"), &with_lemnos(), None);
    let sessions = sessions(
        Lease {
            period: PERIOD,
            renew: RENEW,
            in_force: false,
        },
        &clock,
    );
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let mut hold = handle.hold();
    let lapses = handle.lapses();
    clock.advance(PERIOD * 100);
    let_turns_run().await;
    assert!(
        lapses.has_changed().map(|changed| !changed).unwrap_or(true),
        "no lease in force: no lapse is recorded"
    );
    assert!(!hold.lapsed(), "the hold never sees a lapse");
    hold.acknowledge();
    assert_eq!(
        renewals(&node),
        0,
        "nothing is sent to the bridge for the lease"
    );
    assert_eq!(
        node.calls_received(),
        0,
        "no request of any kind reaches the node"
    );
}
