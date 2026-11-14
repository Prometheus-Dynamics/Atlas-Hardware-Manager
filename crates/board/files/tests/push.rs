//! Pushes of a file to a simulated Orion node, through a relay that throttles, cuts and refuses.
//!
//! The always-on proof is a 16 MiB push with three forced drops. The 1 GiB push is the same with
//! three drops at the same kinds of point, and is `#[ignore]`d for its length. Every drop is
//! forced on a progress point the board reported, not on a clock, and every check holds whether or
//! not the session also dropped on its own: the monitor's heartbeat can drop a session under load,
//! and the resume must cope with that too.

#![cfg(target_os = "linux")]

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use atlas_files::{Counters, Progress, Push, PushError, PushKind, Stage, submit};
use atlas_jobs::{Backoff, DeviceHolds, Events, JobId, JobKind, JobStatus, JobTiming, Jobs};
use atlas_session::{Clock, Handle, Lease, Pause, Presence, Sessions};
use atlas_sim_node::{Hello, Kind, Mode, SimNode};
use common::{DEADLINE, Generated, POLL, Relay, is_generated};
use orion::{Address, Identity, Options, Orion};

/// Orion's window: eight chunks of 32 KiB in flight, and the one being read.
const CHUNK: u64 = 32 * 1024;
const WINDOW_BYTES: u64 = 9 * CHUNK;

/// The monitor's wait between tries, short so a test that waits on a session does not wait long.
#[derive(Debug)]
struct ShortPause;

impl Pause for ShortPause {
    fn pause(&self, delay: Duration, stop: &std::sync::atomic::AtomicBool) {
        let end = Instant::now() + delay.min(Duration::from_millis(10));
        while !stop.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(POLL);
        }
    }
}

/// A node, a relay in front of it at `bytes_per_sec`, and the jobs that run pushes.
struct Rig {
    node: SimNode,
    relay: Relay,
    handle: Handle,
    jobs: Jobs,
    kind: Arc<PushKind>,
    _sessions: Sessions,
    _scratch: tempfile::TempDir,
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("files-{name}"))
}

async fn rig(name: &str, bytes_per_sec: u64) -> Rig {
    let dir = scratch(name);
    let _ = std::fs::remove_dir_all(&dir);
    let node = SimNode::start(&dir).expect("the simulated node starts");
    let relay = Relay::start(node.network(), bytes_per_sec);
    let sessions = Sessions::new(Arc::new(ShortPause));
    let address = Address::from_str(&format!("orion+tcp://{}", relay.addr())).unwrap();
    let handle = sessions.handle(&address).expect("a handle");
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected through the relay")
    .expect("the monitor runs");

    let scratch = tempfile::Builder::new()
        .prefix("jobs-")
        .tempdir_in(Path::new(env!("CARGO_TARGET_TMPDIR")))
        .expect("a scratch directory");
    let kind = PushKind::new();
    let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone()];
    let timing = JobTiming {
        step_timeout: Duration::from_secs(24 * 60 * 60),
        backoff: Backoff {
            first: Duration::from_millis(10),
            cap: Duration::from_secs(5),
        },
    };
    let jobs = Jobs::open(
        scratch.path().join("jobs.json"),
        kinds,
        Events::new(1024),
        timing,
        DeviceHolds::new(),
    )
    .await
    .expect("the jobs open");
    Rig {
        node,
        relay,
        handle,
        jobs,
        kind,
        _sessions: sessions,
        _scratch: scratch,
    }
}

/// Waits until the push's progress satisfies `check`, and returns it.
async fn progress_where(push: &Push, what: &str, check: impl Fn(&Progress) -> bool) -> Progress {
    let mut progress = push.watch();
    let found = tokio::time::timeout(DEADLINE, progress.wait_for(|now| check(now)))
        .await
        .unwrap_or_else(|_| panic!("{what} never happened"))
        .expect("the push is running");
    (*found).clone()
}

/// Waits until the job is in a status that `check` accepts, and returns it.
async fn job_where(jobs: &Jobs, id: JobId, what: &str, check: impl Fn(&JobStatus) -> bool) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(job) = jobs.get(id)
            && check(&job.status)
        {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(POLL).await;
    }
}

/// Waits until the session is connected again.
async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected")
    .expect("the monitor runs");
}

/// The bytes that were sent again after a session dropped are only those in the window, once for
/// each attempt that began after a drop. The board never had to be sent what it acknowledged.
fn assert_resumed_within_the_window(counters: Counters, size: u64) {
    assert!(
        counters.bytes_sent >= size,
        "the whole file was sent: {counters:?}"
    );
    let resent = counters.bytes_sent - size;
    let drops = counters.attempts.saturating_sub(1);
    assert!(
        resent <= drops * WINDOW_BYTES,
        "bytes sent again ({resent}) beyond one window per drop ({drops} drops): {counters:?}"
    );
    assert!(
        counters.max_in_flight <= WINDOW_BYTES,
        "more than Orion's window was unacknowledged at once: {counters:?}"
    );
}

/// Pushes a generated file of `size` bytes, cutting the link at each fraction of it that the board
/// has acknowledged, and waits for it to commit. Returns the push's counters.
async fn push_through_drops(rig: &Rig, size: u64, seed: u64, cuts: &[f64]) -> Counters {
    let destination = "images/system.img";
    let push = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, seed)),
    )
    .await
    .expect("the push starts");
    for fraction in cuts {
        let at = (size as f64 * fraction) as u64;
        progress_where(&push, "the board holds the cut point", |p| p.sent >= at).await;
        rig.relay.cut();
    }
    job_where(&rig.jobs, push.job(), "the push commits", |status| {
        *status == JobStatus::Done
    })
    .await;

    let progress = push.progress();
    assert_eq!(progress.stage, Stage::Committed, "{progress:?}");
    assert_eq!(progress.sent, size);
    let committed = rig.node.shelf().committed.lock().unwrap();
    let data = committed
        .get(destination)
        .expect("the board holds the file");
    assert!(
        is_generated(data, size, seed),
        "the board holds exactly the bytes that were pushed"
    );
    push.counters()
}

#[tokio::test]
async fn a_push_lands_whole_and_the_board_holds_exactly_what_was_pushed() {
    let rig = rig("whole", 0).await;
    let size = 2 * 1024 * 1024;
    let counters = push_through_drops(&rig, size, 1, &[]).await;
    assert_resumed_within_the_window(counters, size);
}

#[tokio::test]
async fn a_drop_mid_push_resumes_without_resending_what_the_board_holds() {
    // Throttled to 16 MiB a second, so the cuts land inside the push.
    let rig = rig("drops", 16 * 1024 * 1024).await;
    let size = 16 * 1024 * 1024;
    let counters = push_through_drops(&rig, size, 2, &[0.25, 0.5, 0.75]).await;
    assert!(
        counters.attempts >= 2,
        "the link was cut, so the push began again on a new session: {counters:?}"
    );
    assert_resumed_within_the_window(counters, size);
}

#[tokio::test]
async fn a_lost_session_is_waited_for_and_the_job_stays_running_not_failed() {
    let rig = rig("waiting", 16 * 1024 * 1024).await;
    let size = 8 * 1024 * 1024;
    let destination = "images/waited.img";
    let push = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, 3)),
    )
    .await
    .expect("the push starts");
    progress_where(&push, "the board holds a quarter", |p| p.sent >= size / 4).await;

    // The node cannot be reached: every new connection is refused, and the live one is cut.
    rig.relay.refuse(true);
    rig.relay.cut();
    progress_where(&push, "the push is waiting for the board", |p| {
        matches!(p.stage, Stage::WaitingForBoard { .. })
    })
    .await;
    assert_eq!(
        rig.jobs.get(push.job()).map(|job| job.status),
        Some(JobStatus::Running),
        "waiting for the board is not a failure: the job is still running"
    );

    // The node comes back: the push goes on from what the board holds.
    rig.relay.refuse(false);
    job_where(&rig.jobs, push.job(), "the push commits", |status| {
        *status == JobStatus::Done
    })
    .await;
    assert_eq!(push.progress().stage, Stage::Committed);
    let committed = rig.node.shelf().committed.lock().unwrap();
    assert!(is_generated(
        committed
            .get(destination)
            .expect("the board holds the file"),
        size,
        3
    ));
    assert_resumed_within_the_window(push.counters(), size);
}

#[tokio::test]
async fn a_cancelled_push_stops_and_the_board_gets_no_more_of_the_file() {
    let rig = rig("cancel", 8 * 1024 * 1024).await;
    let size = 8 * 1024 * 1024;
    let destination = "images/cancelled.img";
    let push = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, 4)),
    )
    .await
    .expect("the push starts");
    progress_where(&push, "the board holds a quarter", |p| p.sent >= size / 4).await;

    assert!(push.cancel(), "the push was still running");
    job_where(&rig.jobs, push.job(), "the job is cancelled", |status| {
        *status == JobStatus::Cancelled
    })
    .await;
    assert_eq!(push.progress().stage, Stage::Cancelled);
    assert!(
        push.counters().bytes_sent < size,
        "the push stopped before it read the whole file: {:?}",
        push.counters()
    );
    assert!(
        rig.node
            .shelf()
            .committed
            .lock()
            .unwrap()
            .get(destination)
            .is_none(),
        "a cancelled push never commits"
    );

    // The session is free again once the push's call has ended: a call answers.
    // Cancelling ends the session the push ran on, so the monitor connects again: a call answers
    // once it is back. A call made in the gap says the node is not connected, and is asked again.
    let deadline = Instant::now() + DEADLINE;
    loop {
        connected(&rig.handle).await;
        let answer = rig
            .handle
            .with_client(|orion| -> Result<String, orion::Error> { Ok(orion.node()?.name) })
            .await;
        if answer.is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the node never answered: {answer:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

#[tokio::test]
async fn a_changed_source_is_refused_and_nothing_more_is_sent() {
    let rig = rig("changed", 8 * 1024 * 1024).await;
    let size = 4 * 1024 * 1024;
    let path = scratch("changed-source.bin");
    write_file(&path, 0x11, size);
    let destination = "images/changed.img";
    let push = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(std::fs::File::open(&path).expect("the file opens")),
    )
    .await
    .expect("the push starts");
    progress_where(&push, "the board holds a part", |p| p.sent >= size / 2).await;

    rig.relay.refuse(true);
    rig.relay.cut();
    progress_where(&push, "the push is waiting", |p| {
        matches!(p.stage, Stage::WaitingForBoard { .. })
    })
    .await;

    // The file changes under the push: same size, other bytes.
    write_file(&path, 0x22, size);
    rig.relay.refuse(false);

    job_where(&rig.jobs, push.job(), "the push is refused", |status| {
        matches!(status, JobStatus::Failed { .. })
    })
    .await;
    let reason = match rig.jobs.get(push.job()).map(|job| job.status) {
        Some(JobStatus::Failed { reason }) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert!(reason.contains("changed"), "{reason}");
    assert!(matches!(push.progress().stage, Stage::Failed { .. }));
    assert!(
        rig.node
            .shelf()
            .committed
            .lock()
            .unwrap()
            .get(destination)
            .is_none(),
        "a changed file is never committed"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn a_different_file_under_the_same_name_starts_from_nothing() {
    let rig = rig("same-name", 8 * 1024 * 1024).await;
    let size = 4 * 1024 * 1024;
    let destination = "images/same-name.img";

    // A part of one file is on the board under the name...
    let first = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, 10)),
    )
    .await
    .expect("the first push starts");
    progress_where(&first, "the board holds half of the first file", |p| {
        p.sent >= size / 2
    })
    .await;
    assert!(first.cancel());
    job_where(&rig.jobs, first.job(), "the first is cancelled", |status| {
        *status == JobStatus::Cancelled
    })
    .await;

    // ...and another file is pushed under the same name. It is a different transfer: it is sent
    // from its own start, never added to the other file's bytes.
    let second = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, 11)),
    )
    .await
    .expect("the second push starts");
    job_where(
        &rig.jobs,
        second.job(),
        "the second push commits",
        |status| *status == JobStatus::Done,
    )
    .await;
    assert!(
        second.counters().bytes_sent >= size,
        "the second file was sent whole, not from the first file's part: {:?}",
        second.counters()
    );
    let committed = rig.node.shelf().committed.lock().unwrap();
    assert!(
        is_generated(committed.get(destination).expect("held"), size, 11),
        "the board holds the second file, not the first file's part and the second's bytes"
    );
}

#[tokio::test]
async fn a_name_the_board_does_not_take_is_refused_before_anything_starts() {
    let rig = rig("name", 0).await;
    let refused = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        "../escape.img",
        Box::new(Generated::new(1024, 5)),
    )
    .await;
    assert!(
        matches!(refused, Err(PushError::Destination(_))),
        "{refused:?}"
    );
    assert!(rig.jobs.list().is_empty(), "no job was made");
}

/// The one gibibyte: 1 GiB generated, over a link throttled to 64 MiB a second, cut three times.
/// Run with `cargo test -p atlas-files --test push -- --ignored --nocapture`. The result is in the
/// PR, not a claim about every run.
#[tokio::test]
#[ignore = "1 GiB over a throttled link: run on demand with --ignored"]
async fn one_gibibyte_over_a_throttled_link_is_interrupted_and_resumed() {
    let rig = rig("gib", 64 * 1024 * 1024).await;
    let size = 1024 * 1024 * 1024;
    let began = Instant::now();
    let counters = push_through_drops(&rig, size, 42, &[0.2, 0.5, 0.8]).await;
    let elapsed = began.elapsed();
    assert!(counters.attempts >= 2, "{counters:?}");
    assert_resumed_within_the_window(counters, size);
    let mut out = std::io::stderr();
    let _ = writeln!(
        out,
        "1 GiB push: {:.1} s wall, attempts {}, bytes read to send {} (resent {}), max in flight {}",
        elapsed.as_secs_f64(),
        counters.attempts,
        counters.bytes_sent,
        counters.bytes_sent - size,
        counters.max_in_flight
    );
}

fn write_file(path: &Path, fill: u8, size: u64) {
    let block = vec![fill; 64 * 1024];
    let mut file = std::fs::File::create(path).expect("the file is created");
    let mut left = size;
    while left > 0 {
        let take = usize::try_from(left).unwrap_or(usize::MAX).min(block.len());
        file.write_all(&block[..take]).expect("the file is written");
        left -= take as u64;
    }
    file.sync_all().expect("the file is synced");
}

/// The key a node at `port` shows, as a person's tool would learn it: one unpinned signed connect.
fn learn_key(port: u16) -> [u8; 32] {
    let address = Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap();
    let options = Options {
        identity: Some(Identity::generate()),
        ..Options::default()
    };
    Orion::connect_with(&address, "push-test", options)
        .expect("the node answers a signed connect")
        .node()
        .expect("the hello")
        .public_key
}

#[tokio::test]
async fn a_push_is_not_continued_on_another_board_and_the_job_fails() {
    let rig = rig("replaced", 4 * 1024 * 1024).await;
    let size = 8 * 1024 * 1024;
    let destination = "images/replaced.img";
    let push = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, 5)),
    )
    .await
    .expect("the push starts");
    progress_where(&push, "the board holds a quarter", |p| p.sent >= size / 4).await;

    // The first board goes: its address is refused, and the live session is cut.
    rig.relay.refuse(true);
    rig.relay.cut();
    progress_where(&push, "the push is waiting for the board", |p| {
        matches!(p.stage, Stage::WaitingForBoard { .. })
    })
    .await;

    // Another board, with another key, comes up on the same port the relay forwards to.
    let port = rig.node.network().port();
    drop(rig.node);
    let other_dir = scratch("replaced-other");
    let _ = std::fs::remove_dir_all(&other_dir);
    let other = SimNode::start_with(&other_dir, &Kind::default(), Some(port))
        .expect("the other board starts");
    rig.relay.refuse(false);

    // The push says another board answers, and sends that board nothing.
    progress_where(&push, "the push says another board answers", |p| {
        matches!(&p.stage, Stage::WaitingForBoard { reason } if reason.contains("another board answers"))
    })
    .await;
    assert_eq!(
        rig.jobs.get(push.job()).map(|job| job.status),
        Some(JobStatus::Running),
        "waiting for another board is not yet a failure"
    );
    assert_eq!(
        other.calls_received(),
        0,
        "the other board received no call"
    );
    assert!(
        !other
            .shelf()
            .committed
            .lock()
            .unwrap()
            .contains_key(destination),
        "the other board holds none of the file"
    );

    // A person accepts the other board's key for this address. The push does not continue on it.
    let address = Address::from_str(&format!("orion+tcp://{}", rig.relay.addr()))
        .unwrap()
        .to_string();
    assert!(rig._sessions.accept_key(&address, learn_key(port)));
    job_where(&rig.jobs, push.job(), "the push ends as failed", |status| {
        matches!(status, JobStatus::Failed { .. })
    })
    .await;
    let status = rig.jobs.get(push.job()).map(|job| job.status);
    match status {
        Some(JobStatus::Failed { reason }) => assert!(
            reason.contains("replaced by another one"),
            "the reason says the board was replaced: {reason}"
        ),
        other => panic!("expected the job failed, got {other:?}"),
    }
    assert_ne!(push.progress().stage, Stage::Committed);
    assert_eq!(
        other.calls_received(),
        0,
        "still no call on the other board"
    );
    assert!(
        !other
            .shelf()
            .committed
            .lock()
            .unwrap()
            .contains_key(destination),
        "the other board still holds none of the file"
    );
}

#[tokio::test]
async fn a_call_issued_during_a_push_completes_while_the_file_is_still_sent() {
    let rig = rig("calls-during", 2 * 1024 * 1024).await;
    let size = 8 * 1024 * 1024;
    let destination = "images/during.img";
    let push = submit(
        &rig.kind,
        &rig.jobs,
        &rig.handle,
        destination,
        Box::new(Generated::new(size, 21)),
    )
    .await
    .expect("the push starts");
    progress_where(&push, "the push is moving", |p| p.sent >= size / 4).await;

    // A call with a wait long enough for the whole push, so only the session decides when it runs.
    let patient = rig.handle.waiting(DEADLINE);
    let answered = patient
        .with_client(|orion| orion.node().map(|node| node.name))
        .await;
    assert!(answered.is_ok(), "the call is answered: {answered:?}");
    let committed_when_answered = rig
        .node
        .shelf()
        .committed
        .lock()
        .unwrap()
        .contains_key(destination);
    // The stepped push (Orion R2) lets the session carry the call between the push's steps.
    assert!(
        !committed_when_answered,
        "the call completes while the file is still being sent"
    );
    job_where(&rig.jobs, push.job(), "the push commits", |status| {
        *status == JobStatus::Done
    })
    .await;
}

/// A push whose session is silent past the lease. The board drops the partial file: its node goes
/// down, and a board that comes back holds no partial. The push waits for the board, notes the lapse
/// as its own stage until bytes are acknowledged again, sends the file again from zero, and commits
/// the right bytes. Everything is resent: the board's dropped partial is not resumed.
///
/// Time is the test's: the lease is read on a manual clock, and the silence past the lease is an
/// `advance` of that clock while the board is down. The relay's rate is cut to a trickle once the
/// quarter is held, so the push cannot finish while the test thread is slow to act.
#[tokio::test]
async fn a_push_silent_past_the_lease_is_sent_again_from_zero_and_says_why() {
    let dir = scratch("lapse");
    let _ = std::fs::remove_dir_all(&dir);
    let board = Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    });
    let node = SimNode::start_with(&dir.join("node"), &board, None).expect("the node starts");
    let port = node.network().port();
    let fast = 8 * 1024 * 1024;
    let relay = Relay::start(node.network(), fast);
    // A lease in force with no renewals from this test's clock: only the advance makes a silence.
    let clock = Clock::manual();
    let sessions = Sessions::new(Arc::new(ShortPause))
        .with_lease(Lease {
            period: Duration::from_secs(60),
            renew: Duration::from_secs(3600),
            in_force: true,
        })
        .with_clock(clock.clone());
    let address = Address::from_str(&format!("orion+tcp://{}", relay.addr())).unwrap();
    let handle = sessions.handle(&address).expect("a handle");
    connected(&handle).await;

    let scratch = tempfile::Builder::new()
        .prefix("jobs-")
        .tempdir_in(Path::new(env!("CARGO_TARGET_TMPDIR")))
        .expect("a scratch directory");
    let kind = PushKind::new();
    let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone()];
    let timing = JobTiming {
        step_timeout: Duration::from_secs(24 * 60 * 60),
        backoff: Backoff {
            first: Duration::from_millis(10),
            cap: Duration::from_secs(5),
        },
    };
    let jobs = Jobs::open(
        scratch.path().join("jobs.json"),
        kinds,
        Events::new(1024),
        timing,
        DeviceHolds::new(),
    )
    .await
    .expect("the jobs open");

    let size = 64 * 1024 * 1024;
    let destination = "images/lapsed.img";
    let push = submit(
        &kind,
        &jobs,
        &handle,
        destination,
        Box::new(Generated::new(size, 7)),
    )
    .await
    .expect("the push starts");
    progress_where(&push, "the board holds a quarter", |p| {
        p.sent >= size / 4 && p.sent < size
    })
    .await;
    let held_before = push.progress().sent;

    // The board drops the transfer: the rate falls to a trickle so nothing more lands, and the node
    // goes down, so the session is lost.
    relay.set_rate(1024);
    drop(node);
    progress_where(&push, "the push waits for the board", |p| {
        matches!(p.stage, Stage::WaitingForBoard { .. })
    })
    .await;
    // Silent past the lease, with the hold on the board.
    clock.advance(Duration::from_secs(61));

    // The board is back, holding nothing, at full speed again.
    relay.set_rate(fast);
    let back =
        SimNode::start_with(&dir.join("node"), &board, Some(port)).expect("the node is back");
    progress_where(&push, "the push says the file starts over", |p| {
        matches!(p.stage, Stage::Resending { .. })
    })
    .await;
    job_where(&jobs, push.job(), "the push commits", |status| {
        *status == JobStatus::Done
    })
    .await;
    assert_eq!(push.progress().stage, Stage::Committed);
    {
        let committed = back.shelf().committed.lock().unwrap();
        assert!(
            is_generated(
                committed
                    .get(destination)
                    .expect("the board holds the file"),
                size,
                7
            ),
            "the committed bytes are the file"
        );
    }
    let counters = push.counters();
    assert!(counters.attempts >= 2, "the file began again: {counters:?}");
    assert!(
        counters.bytes_sent >= held_before + size,
        "everything was resent: the board's partial was not resumed: {counters:?} (held {held_before})"
    );
}
