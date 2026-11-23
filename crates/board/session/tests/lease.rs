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

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_session::double::{Double, Event, Log};
use atlas_session::{Cancel, Clock, Handle, Lease, Pause, Presence, Sessions, SteppedTransfer};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use orion::{Address, Orion};
mod common;
use common::scratch;

/// The monitor's wait between tries: it only yields, so a node that is gone is not spun on.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
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
    // The first renewal is due at once: its describe reply gives the board's lease period.
    renewed(
        &handle,
        &node,
        1,
        "the first renewal, reading the board's period",
    )
    .await;
    let_turns_run().await;
    assert_eq!(
        renewals(&node),
        1,
        "the clock has not moved: nothing else is due"
    );

    clock.advance(RENEW);
    renewed(
        &handle,
        &node,
        2,
        "a renewal once the renew period has passed",
    )
    .await;
    clock.advance(RENEW);
    renewed(&handle, &node, 3, "a second renewal").await;
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
    renewed(
        &handle,
        &node,
        1,
        "the first renewal, reading the board's period",
    )
    .await;
    for _ in 0..20 {
        clock.advance(RENEW / 2);
        bridge_read(&handle).await;
    }
    let_turns_run().await;
    assert_eq!(
        renewals(&node),
        1,
        "the holder's own requests renewed the lease, so none was sent after the first"
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
    renewed(
        &handle,
        &node,
        1,
        "the first renewal, reading the board's period",
    )
    .await;
    clock.advance(RENEW);
    renewed(&handle, &node, 2, "a renewal with both holds").await;

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

#[tokio::test]
async fn a_renewal_the_bridge_refused_is_not_counted_and_is_tried_again_on_the_backoff() {
    let clock = Clock::manual();
    let node = start_node(&scratch("renew-refused"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // Refused from the start: the first renewal (due at once, to read the period) is refused too.
    node.refuse_describe(true);
    let _hold = handle.hold();
    renewed(&handle, &node, 1, "the first renewal, refused").await;
    // The refused renewal is not a renewal: the session tries again well before a full renew period.
    node.refuse_describe(false);
    clock.advance(Duration::from_secs(1));
    renewed(&handle, &node, 2, "a second renewal on the backoff").await;
}

#[tokio::test]
async fn a_bridge_request_the_node_did_not_answer_does_not_renew_the_lease() {
    let clock = Clock::manual();
    let node = start_node(&scratch("job-refused"), &with_lemnos(), None);
    let sessions = sessions(lease(), &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold();
    renewed(
        &handle,
        &node,
        1,
        "the first renewal, reading the board's period",
    )
    .await;
    clock.advance(RENEW / 2);
    // A bridge request that fails before the node answers it: it must not count as the renewal.
    let _ = handle
        .with_bridge(|_orion: &Orion| {
            Err::<(), _>(orion::Error::Client(orion::ClientError::Denied(
                "refused before the node answered",
            )))
        })
        .await;
    settled(&handle).await;
    clock.advance(RENEW / 2);
    renewed(
        &handle,
        &node,
        2,
        "the renewal, due a full period after the last answered request",
    )
    .await;
}

/// The locked board's own lease (`Lease::DEFAULT`, now in force): a hold is renewed on each renew period,
/// so the board, which drops a caller silent for its period (60 s), drops nothing while the hold lasts.
#[tokio::test]
async fn the_locked_board_lease_is_kept_alive_by_a_hold_and_the_board_drops_nothing() {
    let clock = Clock::manual();
    let node = start_node(&scratch("locked-default"), &with_lemnos(), None);
    let sessions = sessions(Lease::DEFAULT, &clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let hold = handle.hold();
    renewed(
        &handle,
        &node,
        1,
        "the first renewal, reading the board's period",
    )
    .await;
    // Four renew periods of silence on the board's clock and the session's clock, in step: without the
    // renewals the board would have dropped the caller at its first full period.
    for step in 1..=4 {
        node.lease_advance(20);
        clock.advance(Duration::from_secs(20));
        renewed(&handle, &node, 1 + step, "a renewal each renew period").await;
    }
    let_turns_run().await;
    assert_eq!(
        node.lease_drops(),
        0,
        "the board dropped nothing: the lease was renewed"
    );
    assert!(!hold.lapsed(), "the hold did not lapse");
}

/// The board's own period (its describe reply's `lease_seconds`) is the one the session keeps, and a
/// silence past it with a hold is a lapse, even though the renew period has not passed.
#[tokio::test]
async fn the_boards_lease_period_is_read_and_a_silence_past_it_is_a_lapse() {
    let clock = Clock::manual();
    let node = start_node(&scratch("period-read"), &with_lemnos(), None);
    node.lease_set(Duration::from_secs(10), Duration::from_secs(600));
    let sessions = sessions(
        Lease {
            period: PERIOD,
            renew: Duration::from_secs(20),
            in_force: true,
        },
        &clock,
    );
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let hold = handle.hold();
    renewed(
        &handle,
        &node,
        1,
        "the first renewal, reading the board's period",
    )
    .await;
    assert_eq!(
        hold.lease_period(),
        Duration::from_secs(10),
        "the board's period of 10 s, not the 60 s default"
    );
    let mut lapses = handle.lapses();
    // 11 s of silence: past the board's 10 s, short of the 20 s renew period, so only the lapse is due.
    clock.advance(Duration::from_secs(11));
    tokio::time::timeout(DEADLINE, lapses.changed())
        .await
        .expect("the silence past the board's period is a lapse")
        .expect("the monitor is running");
    assert!(hold.lapsed(), "the hold sees the lapse");
}
