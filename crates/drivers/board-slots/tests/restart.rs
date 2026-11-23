//! A restart the board was asked for, followed by the board's state: the session ending and the
//! board answering again on a new one. Raze #2, 2026-11-23: the restart's own answer was lost, or
//! ended as failed ("the program that runs it went away"), while the board restarted as asked.

mod common;

use std::time::{Duration, Instant};

use atlas_actions::{BoardActions, Restart, follow_restart, restart_in_doubt};
use atlas_sim_node::script::{Behaviour, PROVIDER_GONE};
use atlas_sim_node::{DEADLINE, names};
use common::Rig;
use tokio::time;

async fn restart(rig: &Rig) -> (u64, Instant, atlas_actions::ActionError) {
    let actions = BoardActions::new(rig.handle.clone());
    let catalog = actions
        .catalog()
        .await
        .expect("the board lists its actions");
    let entry = catalog
        .get(names::REBOOT)
        .expect("the board offers the restart")
        .clone();
    let asked = (rig.handle.session(), Instant::now());
    let error = actions
        .run(&entry, &[])
        .await
        .expect_err("the restart's answer does not come back");
    (asked.0, asked.1, error)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_whose_answer_went_with_it_is_seen_by_the_board_coming_back() {
    let rig = Rig::start("restart-back").await;
    rig.node
        .script()
        .push_action(names::REBOOT, Behaviour::AnswerLost);
    let (asked, since, error) = restart(&rig).await;
    assert!(error.to_string().contains(PROVIDER_GONE), "{error}");
    assert!(restart_in_doubt(&error), "{error:?}");

    let handle = rig.handle.clone();
    let follow = tokio::spawn(async move { follow_restart(&handle, asked, since, DEADLINE).await });
    rig.relay.away(true);
    rig.relay.cut();
    time::sleep(Duration::from_millis(50)).await;
    rig.relay.away(false);
    let seen = time::timeout(DEADLINE, follow)
        .await
        .expect("the board's return is seen")
        .expect("the follow ends");
    assert!(matches!(seen, Restart::Back { .. }), "{seen:?}");
    assert!(
        seen.words().starts_with("the board restarted"),
        "{}",
        seen.words()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_that_failed_and_left_the_session_up_did_not_restart() {
    let rig = Rig::start("restart-stayed").await;
    rig.node.script().push_action(
        names::REBOOT,
        Behaviour::Fail("reboot: permission denied".into()),
    );
    let (asked, since, error) = restart(&rig).await;
    assert!(
        restart_in_doubt(&error),
        "a failure after the send leaves it open: {error:?}"
    );
    let seen = follow_restart(&rig.handle, asked, since, Duration::from_millis(300)).await;
    assert!(matches!(seen, Restart::Stayed(_)), "{seen:?}");
    assert!(
        seen.words().starts_with("the board did not restart"),
        "{}",
        seen.words()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_restart_is_its_answer() {
    let rig = Rig::start("restart-refused").await;
    rig.node.script().push_action(
        names::REBOOT,
        Behaviour::Refuse("BUSY: an install is running".into()),
    );
    let (_, _, error) = restart(&rig).await;
    assert!(!restart_in_doubt(&error), "{error:?}");
}
