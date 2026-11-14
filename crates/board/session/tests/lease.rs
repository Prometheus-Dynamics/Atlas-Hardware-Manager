//! The lease: a session keeps a board's lease alive while it holds something on the board and nothing
//! of its own is running there, sends nothing for it otherwise, and reports a lapse from the board's
//! note or its `leases.ended` fact. The node is the simulated one, which counts the requests it
//! receives and keeps its own lease clock (`lease_advance`).
//!
//! Time is the test's, not the machine's: the session measures its lease on a manual clock
//! (`atlas_session::Clock::manual`), which moves only when a test advances it. The board's period is
//! read from its hello fact (`lease.seconds`, 60 s in the simulated node), so a renewal is due at a
//! third of it: 20 s of clock time. The waits that remain are negative checks ("nothing is sent"),
//! which a stall can only make pass more easily, and waits for an event the test has already caused,
//! with the generous `DEADLINE`.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use atlas_session::{Clock, Handle, Pause, Presence, Sessions, lapse_words};
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

/// The simulated board's lease period (its `lease.seconds`), and the renewal due at a third of it.
const RENEW: Duration = Duration::from_secs(20);

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

/// A board with no lease: no `lease.seconds` fact, no `leases.ended`.
fn without_lease() -> Kind {
    Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0002".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        lease_seconds: None,
        ..Hello::default()
    })
}

fn start_node(dir: &Path, kind: &Kind, port: Option<u16>) -> SimNode {
    SimNode::start_with(dir, kind, port).expect("the simulated node starts")
}

fn address(port: u16) -> Address {
    Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap()
}

/// The requests the node took that renew a lease: its bridge `.call`s, including the renewal.
fn renewals(node: &SimNode) -> usize {
    node.bridge_calls_started()
}

fn sessions(clock: &Clock) -> Sessions {
    Sessions::new(Arc::new(NoPause)).with_clock(clock.clone())
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

async fn until(what: &str, check: impl Fn() -> bool) {
    let end = Instant::now() + DEADLINE;
    while !check() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
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

/// Waits until the board's hello document says it has ended `want` leases (the fact is published on
/// the node's own tick, so a test reads it back before the session can).
async fn board_says_ended(handle: &Handle, want: u64) {
    let end = Instant::now() + DEADLINE;
    loop {
        let node = handle
            .with_client(|orion: &Orion| orion.node())
            .await
            .expect("the node answers");
        let said = node
            .facts
            .iter()
            .find(|fact| fact.name == "leases.ended")
            .map(|fact| String::from_utf8_lossy(&fact.value).into_owned());
        if said.as_deref() == Some(want.to_string().as_str()) {
            return;
        }
        assert!(Instant::now() < end, "the board never said leases.ended {want}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn lapses(handle: &Handle) -> u64 {
    *handle.lapses().borrow()
}

#[tokio::test]
async fn with_no_hold_nothing_is_sent_for_the_lease() {
    let clock = Clock::manual();
    let node = start_node(&scratch("no-hold"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    clock.advance(Duration::from_secs(3600));
    let_turns_run().await;
    assert_eq!(renewals(&node), 0, "no hold, so no renewal");
    assert_eq!(node.calls_received(), 0, "no hold, so no request at all");
    assert_eq!(lapses(&handle), 0);
}

#[tokio::test]
async fn an_idle_hold_is_renewed_at_a_third_of_the_board_period() {
    let clock = Clock::manual();
    let node = start_node(&scratch("third"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold("an open terminal");
    let_turns_run().await;
    clock.advance(RENEW - Duration::from_secs(1));
    let_turns_run().await;
    assert_eq!(renewals(&node), 0, "19 s of 60 s is not yet due");

    clock.advance(Duration::from_secs(1));
    renewed(&handle, &node, 1, "the first renewal").await;
    clock.advance(RENEW);
    renewed(&handle, &node, 2, "a second renewal").await;
    assert_eq!(lapses(&handle), 0);
}

#[tokio::test]
async fn a_work_hold_sends_no_renewal_and_its_silence_is_no_lapse() {
    let clock = Clock::manual();
    let node = start_node(&scratch("work"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // A run, install or transfer in flight keeps its caller alive on the board by itself.
    let _work = handle.hold_work("a run on the board");
    let_turns_run().await;
    clock.advance(Duration::from_secs(3600));
    let_turns_run().await;
    assert_eq!(renewals(&node), 0, "work in flight: no renewal is sent");
    assert_eq!(lapses(&handle), 0, "work in flight: its silence is no lapse");
}

#[tokio::test]
async fn a_board_with_no_lease_fact_gets_nothing_for_a_hold() {
    let clock = Clock::manual();
    let node = start_node(&scratch("no-fact"), &without_lease(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold("an open terminal");
    let _work = handle.hold_work("a run on the board");
    clock.advance(Duration::from_secs(3600));
    let_turns_run().await;
    assert_eq!(node.calls_received(), 0, "no lease fact: nothing is sent");
    assert_eq!(lapses(&handle), 0, "no lease fact: no lapse is recorded");
}

#[tokio::test]
async fn two_holds_keep_the_lease_until_both_are_gone() {
    let clock = Clock::manual();
    let node = start_node(&scratch("two"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let first = handle.hold("an open terminal");
    let second = handle.hold("a claimed device");
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "a renewal with both holds").await;

    drop(first);
    let before = renewals(&node);
    clock.advance(RENEW);
    until("a renewal with one hold left", || renewals(&node) > before).await;
    settled(&handle).await;

    drop(second);
    let_turns_run().await;
    let settled_at = renewals(&node);
    clock.advance(RENEW * 3);
    let_turns_run().await;
    assert_eq!(renewals(&node), settled_at, "no hold is left: no renewal");
}

#[tokio::test]
async fn a_holders_own_bridge_requests_are_its_renewals() {
    let clock = Clock::manual();
    let node = start_node(&scratch("traffic"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // Each step moves the clock by 10 s, then the holder makes a bridge request: silence never
    // reaches a third of the period, so the session sends nothing of its own.
    let _hold = handle.hold("an open terminal");
    for _ in 0..20 {
        clock.advance(RENEW / 2);
        bridge_read(&handle).await;
    }
    let_turns_run().await;
    assert_eq!(renewals(&node), 0, "the holder's requests did the renewing");
    assert_eq!(lapses(&handle), 0);
}

#[tokio::test]
async fn a_lapse_seen_by_the_board_fact_alone_is_one_lapse() {
    let clock = Clock::manual();
    let node = start_node(&scratch("ended-alone"), &with_lemnos(), None);
    node.lease_note(false);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let hold = handle.hold("an open terminal");
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "the first renewal, which registers the caller").await;
    // The board drops the silent holder, and this session says nothing for longer than its lease.
    node.lease_advance(61);
    board_says_ended(&handle, 1).await;
    clock.advance(RENEW);
    renewed(&handle, &node, 2, "the renewal that finds the lapse").await;

    assert_eq!(lapses(&handle), 1, "leases.ended moved while the hold existed");
    assert!(hold.lapsed(), "the hold learns of the lapse");
    let words = handle.lapse_words();
    assert!(words.contains("an open terminal"), "{words}");
    assert!(words.contains("nothing was retried"), "{words}");
}

#[tokio::test]
async fn a_lapse_seen_by_the_note_and_the_fact_together_is_reported_once() {
    let clock = Clock::manual();
    let node = start_node(&scratch("both"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold("an open terminal");
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "the first renewal, which registers the caller").await;
    node.lease_advance(61);
    board_says_ended(&handle, 1).await;
    clock.advance(RENEW);
    // The fact is read first (a lapse), then the renewal's reply carries the note (the same lapse).
    renewed(&handle, &node, 1, "the renewal after the drop").await;
    assert_eq!(lapses(&handle), 1, "both signals for one expiry: one lapse");
}

#[tokio::test]
async fn the_note_in_a_reply_is_read_once_and_its_rest_is_kept() {
    let clock = Clock::manual();
    let node = start_node(&scratch("note-read"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    // Only the exact note, or the note then `"; "`, is a lapse. The rest is the reply's own detail.
    assert_eq!(handle.bridge_detail("lease-ended: holds released; the run was refused"), "the run was refused");
    assert_eq!(handle.bridge_detail("lease-ended: holds released"), "");
    assert_eq!(lapses(&handle), 1, "a note is one lapse");
    assert_eq!(
        handle.bridge_detail("the reply said lease-ended: holds released later"),
        "the reply said lease-ended: holds released later"
    );
    assert_eq!(lapses(&handle), 1, "a note that is only contained later is no lapse");

    // Once per expiry: a second note with no request in between is the same expiry.
    assert_eq!(handle.bridge_detail("lease-ended: holds released"), "");
    assert_eq!(lapses(&handle), 1);
    // A request that got through starts a new expiry, and its note is a new lapse.
    bridge_read(&handle).await;
    assert_eq!(handle.bridge_detail("lease-ended: holds released"), "");
    assert_eq!(lapses(&handle), 2);
    assert!(lapse_words(&[]).contains("nothing was retried"));
}

#[tokio::test]
async fn after_a_lapse_nothing_is_claimed_again_or_retried() {
    let clock = Clock::manual();
    let node = start_node(&scratch("no-retry"), &with_lemnos(), None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold("a claimed device");
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "the first renewal, which registers the caller").await;
    node.lease_advance(61);
    board_says_ended(&handle, 1).await;
    clock.advance(RENEW);
    renewed(&handle, &node, 2, "the renewal that finds the lapse").await;

    // The session goes on renewing the still-held lease as before, and nothing else is sent: no
    // retry of a hold, no request of any other kind.
    // Each renewal restarts the silence, so the clock moves a renewal period at a time.
    clock.advance(RENEW);
    renewed(&handle, &node, 3, "the ordinary renewals go on").await;
    clock.advance(RENEW);
    renewed(&handle, &node, 4, "the ordinary renewals go on").await;
    assert_eq!(
        node.calls_received(),
        renewals(&node),
        "only renewals were sent after the lapse"
    );
    assert_eq!(lapses(&handle), 1);
}

#[tokio::test]
async fn a_lost_session_renews_again_on_the_same_board() {
    let clock = Clock::manual();
    let dir = scratch("lost");
    let kind = with_lemnos();
    let node = start_node(&dir, &kind, None);
    let port = node.network().port();
    let sessions = sessions(&clock);
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold("an open terminal");
    clock.advance(RENEW);
    renewed(&handle, &node, 1, "a renewal before the loss").await;
    let key = handle.node_key().expect("connected before the loss");

    drop(node);
    lost(&handle).await;
    let back = start_node(&dir, &kind, Some(port));
    connected(&handle).await;
    assert_eq!(handle.node_key(), Some(key), "the same node is back");

    clock.advance(RENEW);
    renewed(&handle, &back, 1, "a renewal once the same node is back").await;
}

#[tokio::test]
async fn a_node_without_lemnos_has_no_lease_and_gets_nothing() {
    let clock = Clock::manual();
    let node = start_node(&scratch("plain"), &Kind::Plain, None);
    let sessions = sessions(&clock);
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;

    let _hold = handle.hold("an open terminal");
    clock.advance(RENEW * 100);
    let_turns_run().await;
    assert_eq!(node.calls_received(), 0, "a plain Orion node has no bridge");
}
