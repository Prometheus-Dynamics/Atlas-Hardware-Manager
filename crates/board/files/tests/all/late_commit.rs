//! A push whose board takes longer to commit than the session's bound on one answer.
//!
//! The board is Orion's whole test board (its real comm core and node logic, in this process), and
//! the receiver for the file is Orion's held-commit program: it answers each commit `HOLD` after it
//! was asked, and keeps its session pumping meanwhile, so the board's pings are answered throughout.
//! The session's bound on one answer is short (`BOUND`), so the test runs in seconds: a commit held
//! past twelve bounds is what a push under the default reply bound cannot wait for.
//!
//! The second test also runs a call while the commit is held, to an action whose runs Orion's test
//! board never answers (`hold_action`): the call ends at its own short wait, and the push it ran
//! beside still ends committed.

#![cfg(target_os = "linux")]

use std::io::{Cursor, ErrorKind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use atlas_files::{Progress, PushKind, Stage, submit};
use atlas_jobs::{Backoff, DeviceHolds, Events, JobKind, JobTiming, Jobs};
use atlas_session::{Handle, Presence, RealPause, Sessions};
use orion::ClientError;
use orion::actions::{ActionCall, Target};
use orion_test_board::TestBoard;

/// The session's bound on one answer for this test.
const BOUND: Duration = Duration::from_millis(400);
/// The commit is held past twelve bounds (the default reply's commit wait) and then some.
const HOLD: Duration = Duration::from_millis(12 * 400 + 1500);
/// The longest this test waits for the push to end; it ends well before, when it is right.
const DEADLINE: Duration = Duration::from_secs(60);
/// How long the call to the unanswered action waits for its run, as a direct call does.
const CALL_WAIT: Duration = Duration::from_secs(1);

/// The session to the board, connected.
async fn connected(board: &TestBoard) -> (Sessions, Handle) {
    let sessions = Sessions::new(Arc::new(RealPause)).with_reply_bound(BOUND);
    let handle = sessions
        .handle(&board.address())
        .expect("the node is turned on");
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the board is connected")
    .expect("the monitor runs");
    (sessions, handle)
}

/// The jobs a push runs under: no step timeout of their own, so the board's bounds are tested.
async fn jobs(scratch: &tempfile::TempDir, kind: &Arc<PushKind>) -> Jobs {
    let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone()];
    let timing = JobTiming {
        step_timeout: Duration::from_secs(24 * 60 * 60),
        backoff: Backoff {
            first: Duration::from_millis(10),
            cap: Duration::from_secs(5),
        },
    };
    Jobs::open(
        scratch.path().join("jobs.json"),
        kinds,
        Events::new(1024),
        timing,
        DeviceHolds::new(),
    )
    .await
    .expect("the jobs open")
}

/// Waits for the push to end, and says what it ended as.
async fn ended(push: &atlas_files::Push, answered: impl Fn() -> usize) -> Progress {
    let mut progress = push.watch();
    let watching = push.watch();
    tokio::time::timeout(
        DEADLINE,
        progress.wait_for(|p| {
            matches!(
                p.stage,
                Stage::Committed | Stage::Failed { .. } | Stage::Cancelled
            )
        }),
    )
    .await
    .unwrap_or_else(|_| {
        panic!(
            "the push ends; last progress {:?}, held {}",
            watching.borrow().clone(),
            answered()
        )
    })
    .expect("the push is running")
    .clone()
}

/// A push of `size` bytes to a held receiver ends `Committed` while the board answers pings.
#[tokio::test(flavor = "multi_thread")]
async fn a_commit_held_past_twelve_bounds_still_ends_committed() {
    let board = TestBoard::start("late-commit");
    let held = board.hold_commits("slots/", HOLD);
    let (_sessions, handle) = connected(&board).await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let kind = PushKind::new();
    let jobs = jobs(&scratch, &kind).await;

    let data = vec![7_u8; 256 * 1024];
    let began = Instant::now();
    let push = submit(
        &kind,
        &jobs,
        &handle,
        "slots/a.img",
        Box::new(Cursor::new(data)),
    )
    .await
    .expect("the push starts");
    let ended = ended(&push, || held.answered()).await;
    let elapsed = began.elapsed();
    eprintln!(
        "late commit: {:?} after {elapsed:?}, held commits answered {}",
        ended.stage,
        held.answered()
    );
    assert_eq!(
        ended.stage,
        Stage::Committed,
        "a commit held {HOLD:?} against a {BOUND:?} bound ends the push committed: {:?}",
        ended.stage
    );
    // The first attempt carries it: a session cut by the bound would begin again, and the
    // commit would then be held again.
    let counters = push.counters();
    assert_eq!(
        counters.attempts, 1,
        "the push committed on its first attempt, not after a cut: {counters:?}"
    );
    assert!(
        elapsed >= HOLD,
        "the commit really was held: the push ended after {elapsed:?}, before {HOLD:?}"
    );
}

/// While a push's commit is held (the session waiting under the push's long ceiling), a call to
/// an action that never answers ends at the call's own short wait, not at the push's ceiling,
/// and the push still ends `Committed` on its first attempt.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_with_no_answer_ends_at_its_own_bound_while_a_held_push_still_commits() {
    let board = TestBoard::start("late-commit-call");
    let held = board.hold_commits("slots/", HOLD);
    let target = Target::Node("board".parse().expect("a node name"));
    let unanswered = board.hold_action("arm.call", target.clone());
    let (_sessions, handle) = connected(&board).await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let kind = PushKind::new();
    let jobs = jobs(&scratch, &kind).await;

    let data = vec![7_u8; 256 * 1024];
    let began = Instant::now();
    let push = submit(
        &kind,
        &jobs,
        &handle,
        "slots/a.img",
        Box::new(Cursor::new(data)),
    )
    .await
    .expect("the push starts");
    // Wait until every byte is in and the board is holding the commit.
    let mut progress = push.watch();
    tokio::time::timeout(
        DEADLINE,
        progress.wait_for(|p| {
            (p.total > 0 && p.sent >= p.total)
                || matches!(
                    p.stage,
                    Stage::Committed | Stage::Failed { .. } | Stage::Cancelled
                )
        }),
    )
    .await
    .expect("the bytes are sent")
    .expect("the push is running");

    // A short call, as a direct action call is: started, and waited for inside the call.
    let asked = Instant::now();
    let answer = handle
        .with_bridge(move |orion| {
            let mut run = orion.run(&ActionCall {
                name: "arm.call",
                target: target.clone(),
                args: b"",
                deadline_ms: 0,
                detached: false,
            })?;
            Ok(match run.finish(CALL_WAIT) {
                Err(ClientError::Io(error)) if error.kind() == ErrorKind::TimedOut => {
                    let _ = run.cancel();
                    Err("not finished in its wait".to_owned())
                }
                other => Ok(format!("{other:?}")),
            })
        })
        .await;
    let call_took = asked.elapsed();
    eprintln!("unanswered call: {answer:?} after {call_took:?}");
    assert_eq!(
        answer.as_ref().ok().cloned(),
        Some(Err("not finished in its wait".to_owned())),
        "the call to an action that never answers ends at its own wait: {answer:?}"
    );
    assert!(
        call_took >= CALL_WAIT && call_took < CALL_WAIT + Duration::from_secs(3),
        "the call ended at its own {CALL_WAIT:?} wait, not at the push's ceiling: {call_took:?}"
    );
    assert_eq!(unanswered.asked(), 1, "the board was asked to run it once");

    let ended = ended(&push, || held.answered()).await;
    let elapsed = began.elapsed();
    eprintln!(
        "late commit beside a call: {:?} after {elapsed:?}, held commits answered {}",
        ended.stage,
        held.answered()
    );
    assert_eq!(
        ended.stage,
        Stage::Committed,
        "the push beside an unanswered call still ends committed: {:?}",
        ended.stage
    );
    let counters = push.counters();
    assert_eq!(
        counters.attempts, 1,
        "the push committed on its first attempt, not after a cut: {counters:?}"
    );
    assert!(
        elapsed >= HOLD,
        "the commit really was held: the push ended after {elapsed:?}, before {HOLD:?}"
    );
}
