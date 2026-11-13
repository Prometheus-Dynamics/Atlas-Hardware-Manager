//! The board's actions against the simulated node: listed from the board, run directly and as
//! jobs, refused with the board's reason, cancelled and confirmed, followed after a lost session,
//! and never started twice.
//!
//! The waits are on conditions with one generous deadline; no test asserts a fixed delay.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use atlas_actions::{
    Action, ActionError, ActionJobs, ArgKind, BoardActions, CancelState, Catalog, Progress,
    Reported, Route,
};
use atlas_jobs::{Backoff, DeviceHolds, Events, JobId, JobStatus, JobTiming};
use atlas_session::{Handle, Pause, Presence, Sessions};
use atlas_sim_node::actions::DRY_RUN;
use atlas_sim_node::names::SELF_TEST;
use atlas_sim_node::script::Behaviour;
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use atlas_types::DeviceKey;
use orion::Address;
use tokio::sync::oneshot;
use tokio::time;

const FAMILY: &str = "test-family";
const SERIAL: &str = "0123abcd";
/// The reason the simulated node gives for a run its service restart ended.
const RESTARTED: &str = "-4: the service restarted before this finished";

/// A pause that does not wait: the failure rule is tested on the session's side. It only yields,
/// so the monitor thread does not spin while the node is gone.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

/// A scratch directory under the system temp dir, kept short: the node's Unix socket must fit
/// `SUN_LEN`, and a long build path (CI's workspace) would not.
fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("atl-c3-{}-{name}", std::process::id()))
}

fn lemnos_kind() -> Kind {
    Kind::WithLemnos(Hello {
        family: FAMILY.into(),
        serial: SERIAL.into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    })
}

fn start(dir: &Path, kind: &Kind) -> SimNode {
    let _ = std::fs::remove_dir_all(dir);
    SimNode::start_with(dir, kind, None).expect("the simulated node starts")
}

fn address(node: &SimNode) -> Address {
    Address::from_str(&format!("orion+tcp://127.0.0.1:{}", node.network().port())).unwrap()
}

fn device() -> DeviceKey {
    DeviceKey::board(FAMILY, SERIAL).expect("a board key")
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

/// Runs `check` until it holds. The deadline is the sim's own, and it only matters when a run is
/// truly stuck.
async fn until(what: &str, mut check: impl FnMut() -> bool) {
    time::timeout(DEADLINE, async {
        while !check() {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// Waits until the handle's monitor reports the node connected.
async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    time::timeout(
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
    time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Lost(_))),
    )
    .await
    .expect("the node is reported lost")
    .expect("the monitor is running");
}

/// A test's board: a simulated node, the session registry, and the board's actions through it.
struct Rig {
    node: SimNode,
    sessions: Arc<Sessions>,
    address: Address,
    board: BoardActions,
}

impl Rig {
    async fn start(name: &str, kind: &Kind) -> Self {
        let node = start(&scratch(name).join("node"), kind);
        let sessions = Arc::new(Sessions::new(Arc::new(NoPause)));
        let address = address(&node);
        let handle = sessions.handle(&address).expect("the node is on");
        connected(&handle).await;
        Self {
            node,
            sessions,
            address,
            board: BoardActions::new(handle),
        }
    }

    async fn catalog(&self) -> Catalog {
        self.board
            .catalog()
            .await
            .expect("the board lists its actions")
    }
}

async fn jobs(name: &str, sessions: Arc<Sessions>) -> ActionJobs {
    let dir = scratch(name).join("jobs");
    let _ = std::fs::remove_dir_all(&dir);
    ActionJobs::open(
        dir.join("jobs.json"),
        sessions,
        Events::new(64),
        JobTiming {
            step_timeout: DEADLINE,
            backoff: Backoff {
                first: Duration::from_millis(10),
                cap: Duration::from_millis(50),
            },
        },
        DeviceHolds::new(),
    )
    .await
    .expect("the job list opens")
}

fn status(jobs: &ActionJobs, id: JobId) -> JobStatus {
    jobs.jobs()
        .get(id)
        .expect("the job is listed")
        .status
        .clone()
}

async fn wait_status(jobs: &ActionJobs, id: JobId, what: &str, ready: impl Fn(&JobStatus) -> bool) {
    until(what, || ready(&status(jobs, id))).await;
}

/// Words for an action built from its description alone: one sample per declared argument.
fn sample_words(action: &Action) -> Vec<String> {
    match &action.args {
        Reported::NotReported => Vec::new(),
        Reported::Known(args) => args
            .iter()
            .map(|arg| match arg.kind {
                ArgKind::Int => "3".to_owned(),
                ArgKind::Float => "1.5".to_owned(),
                ArgKind::Bool => "on".to_owned(),
                ArgKind::Text | ArgKind::Other(_) => "label".to_owned(),
            })
            .collect(),
    }
}

fn named<'a>(catalog: &'a Catalog, name: &str) -> &'a Action {
    catalog
        .get(name)
        .unwrap_or_else(|| panic!("the board lists {name}"))
}

#[tokio::test]
async fn the_catalog_gives_what_the_board_gives_and_marks_the_rest_not_reported() {
    let rig = Rig::start("catalog", &lemnos_kind()).await;
    let catalog = rig.catalog().await;

    // The board says self-test is long, but gives no label and no arguments.
    let self_test = named(&catalog, SELF_TEST);
    assert_eq!(self_test.label, Reported::NotReported);
    assert_eq!(self_test.args, Reported::NotReported);
    assert_eq!(self_test.long_running, Reported::Known(true));
    assert_eq!(self_test.route(), Route::Job);

    // The board gives everything for locate.
    let locate = named(&catalog, "locate");
    assert_eq!(locate.label, Reported::Known("Find it".into()));
    assert_eq!(locate.args, Reported::Known(Vec::new()));
    assert_eq!(locate.route(), Route::Direct);

    // The dry run has typed arguments and no label: the gap is shown, not filled in.
    let dry = named(&catalog, DRY_RUN);
    assert_eq!(dry.label, Reported::NotReported);
    let args = dry.args.known().expect("the dry run gives its arguments");
    assert_eq!(args[0].kind, ArgKind::Text);
    assert_eq!(args[1].kind, ArgKind::Int);
    assert_eq!(dry.route(), Route::Direct);
}

#[tokio::test]
async fn every_listed_action_runs_on_the_simulated_node() {
    let rig = Rig::start("every", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let jobs = jobs("every", rig.sessions.clone()).await;

    for action in catalog.actions() {
        let argv = sample_words(action);
        match action.route() {
            Route::Direct => {
                let done = rig
                    .board
                    .run(action, &argv)
                    .await
                    .unwrap_or_else(|error| panic!("{}: {error}", action.name));
                if action.name == "locate" {
                    assert_eq!(done.output, b"blinking");
                }
            }
            Route::Job => {
                let id = jobs
                    .submit(device(), &rig.address, action, &argv)
                    .await
                    .unwrap_or_else(|error| panic!("{}: {error}", action.name));
                wait_status(&jobs, id, &action.name, |status| {
                    matches!(status, JobStatus::Done)
                })
                .await;
            }
        }
    }
    assert!(catalog.actions().len() >= 7, "the node lists its actions");
}

#[tokio::test]
async fn an_action_the_code_has_never_heard_of_runs_from_its_description() {
    let rig = Rig::start("described", &lemnos_kind()).await;
    let catalog = rig.catalog().await;

    // Pick by what the board says, not by name: no label, typed arguments.
    let described = catalog
        .actions()
        .iter()
        .find(|action| {
            action.label == Reported::NotReported
                && action.args.known().is_some_and(|args| !args.is_empty())
        })
        .expect("the board describes an action from its arguments alone");
    let argv = sample_words(described);
    assert_eq!(argv, ["label", "3"], "the words follow the declared types");

    let done = rig
        .board
        .run(described, &argv)
        .await
        .expect("the action runs");
    assert_eq!(done.output, b"dry run label: 3 steps");
}

#[tokio::test]
async fn a_refusal_comes_back_with_the_boards_reason_for_a_call_and_for_a_job() {
    let rig = Rig::start("refusal", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let jobs = jobs("refusal", rig.sessions.clone()).await;

    let reason = "no dry runs while the board is in maintenance";
    rig.node
        .script()
        .push_action(DRY_RUN, Behaviour::Refuse(reason.into()));
    let direct = rig
        .board
        .run(named(&catalog, DRY_RUN), &words("label 2"))
        .await
        .unwrap_err();
    assert_eq!(direct, ActionError::Refused(reason.into()));
    assert_eq!(
        direct.board_reason(),
        Some(reason),
        "the words are the board's"
    );

    let refused = "a self-test cannot start while a trial is settling";
    rig.node
        .script()
        .push_action(SELF_TEST, Behaviour::Refuse(refused.into()));
    let id = jobs
        .submit(device(), &rig.address, named(&catalog, SELF_TEST), &[])
        .await
        .expect("the job is queued");
    wait_status(
        &jobs,
        id,
        "the refused job to end",
        |status| matches!(status, JobStatus::Failed { reason } if reason == refused),
    )
    .await;
}

#[tokio::test]
async fn bad_arguments_are_refused_before_anything_is_sent() {
    let rig = Rig::start("badargs", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let jobs = jobs("badargs", rig.sessions.clone()).await;
    let dry = named(&catalog, DRY_RUN);

    // A queued refusal stands in front of the next real send. If a bad call reached the node it
    // would take it, and the next good call would then succeed.
    rig.node
        .script()
        .push_action(DRY_RUN, Behaviour::Refuse("the first real send".into()));
    let not_a_count = rig.board.run(dry, &words("label many")).await.unwrap_err();
    assert!(
        matches!(&not_a_count, ActionError::BadArguments(text) if text.contains("steps")),
        "{not_a_count}"
    );
    let missing = rig.board.run(dry, &words("label")).await.unwrap_err();
    assert!(matches!(missing, ActionError::BadArguments(_)), "{missing}");
    let bad_job = jobs
        .submit(device(), &rig.address, dry, &words("label many"))
        .await
        .unwrap_err();
    assert!(matches!(bad_job, ActionError::BadArguments(_)), "{bad_job}");
    assert!(
        jobs.jobs().list().is_empty(),
        "a refused submit queues nothing"
    );

    let next = rig.board.run(dry, &words("label 2")).await.unwrap_err();
    assert_eq!(next, ActionError::Refused("the first real send".into()));
}

#[tokio::test]
async fn a_cancel_stops_the_run_on_the_board_and_the_board_confirms_it() {
    let rig = Rig::start("cancel", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let jobs = jobs("cancel", rig.sessions.clone()).await;
    let self_test = named(&catalog, SELF_TEST);

    rig.node.script().push_action(SELF_TEST, Behaviour::Stall);
    let id = jobs
        .submit(device(), &rig.address, self_test, &[])
        .await
        .expect("the job is queued");
    until("the run to start and stall on the board", || {
        rig.node.stalled_runs() == 1
    })
    .await;

    assert!(jobs.cancel(id).await, "the active job is cancelled");
    wait_status(&jobs, id, "the job to end cancelled", |status| {
        matches!(status, JobStatus::Cancelled)
    })
    .await;
    assert_eq!(
        jobs.cancel_state(id),
        Some(CancelState::Confirmed),
        "the board ended the run cancelled"
    );

    // The board is free: the same action now runs to its end.
    let again = jobs
        .submit(device(), &rig.address, self_test, &[])
        .await
        .expect("a second job is queued");
    wait_status(&jobs, again, "the second run to finish", |status| {
        matches!(status, JobStatus::Done)
    })
    .await;
}

#[tokio::test]
async fn a_session_lost_during_a_long_action_is_followed_and_never_started_again() {
    let rig = Rig::start("lost", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let jobs = jobs("lost", rig.sessions.clone()).await;
    let self_test = named(&catalog, SELF_TEST);

    // Two stalls queued: a second start would take the second one, so the count shows it.
    rig.node.script().push_action(SELF_TEST, Behaviour::Stall);
    rig.node.script().push_action(SELF_TEST, Behaviour::Stall);
    let id = jobs
        .submit(device(), &rig.address, self_test, &[])
        .await
        .expect("the job is queued");
    until("the run to start and stall", || {
        rig.node.stalled_runs() == 1
    })
    .await;

    // The session goes: the monitor is stopped, so the job cannot reach the node.
    let text = rig.address.to_string();
    rig.sessions.turn_off(&text);
    wait_status(&jobs, id, "the job to wait for the node", |status| {
        matches!(status, JobStatus::Retrying { .. })
    })
    .await;

    // The node comes back. The job re-attaches to the run the board kept (it was detached).
    rig.sessions.turn_on(&text);
    let handle = rig
        .sessions
        .handle(&rig.address)
        .expect("the node is on again");
    connected(&handle).await;

    // The run ends (the service restarts): the followed run's own result reaches the job.
    rig.node.restart_daemon();
    wait_status(
        &jobs,
        id,
        "the followed run's result",
        |status| matches!(status, JobStatus::Failed { reason } if reason == RESTARTED),
    )
    .await;
    assert_eq!(
        rig.node.stalled_runs(),
        1,
        "the run was followed; the action was not started a second time"
    );
}

#[tokio::test]
async fn a_start_the_node_does_not_confirm_is_not_sent_again() {
    // A call's wait of one second: the blocked monitor makes the start's answer late.
    let node = start(&scratch("inflight").join("node"), &lemnos_kind());
    let sessions = Arc::new(Sessions::with_call_wait(
        Arc::new(NoPause),
        Duration::from_secs(1),
    ));
    let handle = sessions.handle(&address(&node)).expect("the node is on");
    connected(&handle).await;
    let board = BoardActions::new(handle.clone());
    let catalog = board.catalog().await.expect("the board lists its actions");
    let dry = named(&catalog, DRY_RUN);

    // Hold the monitor on a call of its own, so the start queued behind it is not answered in time.
    let (release, held) = mpsc::channel::<()>();
    let (holding, is_holding) = oneshot::channel::<()>();
    let blocker = {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle
                .with_client(move |_| -> Result<(), orion::Error> {
                    let _ = holding.send(());
                    let _ = held.recv();
                    Ok(())
                })
                .await
        })
    };
    is_holding
        .await
        .expect("the blocking call is on the monitor");

    // The start the node does not answer must not be sent again, and must not run later.
    node.script().push_action(
        DRY_RUN,
        Behaviour::Refuse("the abandoned start must not run".into()),
    );
    let began = Instant::now();
    let unanswered = board.run(dry, &words("label 2")).await.unwrap_err();
    assert!(
        matches!(&unanswered, ActionError::Unknown { name, reason }
            if name == DRY_RUN && reason.contains("did not answer in time")),
        "{unanswered:?}"
    );
    assert!(began.elapsed() < DEADLINE, "the caller is told at once");

    release.send(()).expect("the blocking call is still there");
    let _ = blocker.await;

    // The abandoned start was never run: the queued refusal is still the next answer.
    let next = board.run(dry, &words("label 2")).await.unwrap_err();
    assert_eq!(
        next,
        ActionError::Refused("the abandoned start must not run".into()),
        "the timed-out start did not run later"
    );
}

#[tokio::test]
async fn a_short_action_that_runs_past_its_wait_is_asked_to_stop() {
    let rig = Rig::start("late", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let dry = named(&catalog, DRY_RUN);

    // A stalled run never finishes on its own: the direct call's wait runs out first.
    rig.node.script().push_action(DRY_RUN, Behaviour::Stall);
    let late = rig.board.run(dry, &words("label 2")).await.unwrap_err();
    assert!(
        matches!(&late, ActionError::NotFinished { name, .. } if name == DRY_RUN),
        "{late:?}"
    );
    assert_eq!(
        rig.node.stalled_runs(),
        1,
        "the run did stall, and was not started twice"
    );

    // The board is not wedged: the next short call runs to its end.
    let next = rig
        .board
        .run(dry, &words("label 2"))
        .await
        .expect("the next short action runs");
    assert_eq!(next.output, b"dry run label: 2 steps");
}

#[tokio::test]
async fn a_start_with_no_connection_sends_nothing() {
    let node = start(&scratch("noconn").join("node"), &lemnos_kind());
    let sessions = Arc::new(Sessions::new(Arc::new(NoPause)));
    let handle = sessions.handle(&address(&node)).expect("the node is on");
    connected(&handle).await;
    let board = BoardActions::new(handle.clone());
    let catalog = board.catalog().await.expect("the board lists its actions");

    drop(node);
    lost(&handle).await;
    let error = board
        .run(named(&catalog, DRY_RUN), &words("label 2"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ActionError::NotConnected(_)),
        "nothing was sent, so the caller may try again: {error:?}"
    );
}

#[tokio::test]
async fn a_plain_orion_node_lists_no_actions_and_says_why() {
    let node = start(&scratch("plain").join("node"), &Kind::Plain);
    let sessions = Arc::new(Sessions::new(Arc::new(NoPause)));
    let handle = sessions.handle(&address(&node)).expect("the node is on");
    connected(&handle).await;
    let error = BoardActions::new(handle).catalog().await.unwrap_err();
    assert!(
        matches!(&error, ActionError::Refused(reason) if !reason.is_empty()),
        "a plain node is a normal node with no list: {error:?}"
    );
}

#[tokio::test]
async fn progress_is_reported_while_a_run_goes_and_ends_with_its_result() {
    let rig = Rig::start("progress", &lemnos_kind()).await;
    let catalog = rig.catalog().await;
    let dry = named(&catalog, DRY_RUN);

    // A hundred steps is two seconds at the node's tick: far longer than one poll.
    let call = rig
        .board
        .start(dry, &words("label 100"))
        .await
        .expect("the run starts");
    let mut seen: Vec<Progress> = Vec::new();
    let done = rig
        .board
        .follow(call, |progress| seen.push(progress.clone()))
        .await
        .expect("the run ends");
    assert_eq!(done.output, b"dry run label: 100 steps");
    assert!(!seen.is_empty(), "the board reported progress");
    assert!(
        seen.windows(2)
            .all(|pair| pair[0].percent <= pair[1].percent),
        "progress does not go backwards: {seen:?}"
    );
    assert!(seen.iter().any(|progress| progress.percent.is_some()));
}
