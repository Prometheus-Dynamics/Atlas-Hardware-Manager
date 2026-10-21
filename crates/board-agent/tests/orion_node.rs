//! board-agent against a real orion-node (in-process, on IPC sockets), with
//! a fake writer (tests/fake-writer.sh) standing in for
//! /usr/lib/board/update: the contract of Orion's docs/device-agent.md.

#![cfg(target_os = "linux")]

mod common;

use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn updates_run_the_writer_and_its_state_is_published() {
    let board = Board::start("update").await;
    let operator = board.operator();

    // After connecting: the writer's state and this boot's id.
    board
        .wait_value(update_action::KEY_STATE, text("idle"))
        .await;
    assert_eq!(
        board.value(update_action::KEY_BOOT_ID),
        Some(text("boot-1"))
    );
    assert_eq!(
        board.value(update_action::KEY_VERSION_ACTIVE),
        Some(text("1.0"))
    );

    // Bad arguments never reach the writer.
    let mut bad = update("bad", "http://atlas/image-2.0.img.xz", SHA);
    bad.args.remove(update_action::ARG_SHA256);
    assert!(rejected(&run(&operator, bad).await, "sha256"));

    // `update` succeeds once staging started; the rest is in the keys:
    // staged, then the agent applies (the fake stops at rebooting).
    let started = run(
        &operator,
        update("u1", "http://atlas/image-2.0.img.xz", SHA),
    )
    .await;
    assert_eq!(started.state, ActionState::Succeeded, "{started:?}");
    assert_eq!(phase(&started), Some(&text(update_action::PHASE_STAGING)));
    board
        .wait_value(update_action::KEY_STATE, text("rebooting"))
        .await;
    assert_eq!(
        board.value(update_action::KEY_VERSION_STAGED),
        Some(text("2.0"))
    );
    assert_eq!(
        board.value(update_action::KEY_PROGRESS),
        Some(TypedConfigValue::UInt(1000))
    );
    let calls = board.calls();
    assert!(
        calls.iter().any(|c| c
            == &format!("stage-url http://atlas/image-2.0.img.xz --sha256 {SHA} --size 1048576")),
        "{calls:?}"
    );
    assert_eq!(calls.last().map(String::as_str), Some("apply"), "{calls:?}");
    board.wait_value("action.u1.state", text("succeeded")).await;

    // A stage that fails: the action started, the keys carry the error.
    let failed = run(&operator, update("u2", "http://atlas/corrupt.img.xz", SHA)).await;
    assert_eq!(failed.state, ActionState::Succeeded);
    board
        .wait_value(update_action::KEY_STATE, text("error"))
        .await;
    board
        .wait_value(
            update_action::KEY_ERROR,
            text("the downloaded image failed its SHA-256 check"),
        )
        .await;

    // A stage in progress: a retry is "already started", another image is
    // refused, cancel stops it.
    let held = run(
        &operator,
        update("u3", "http://atlas/hold/image-3.0.img.xz", SHA),
    )
    .await;
    assert_eq!(phase(&held), Some(&text(update_action::PHASE_STAGING)));
    board
        .wait_value(update_action::KEY_STATE, text("staging"))
        .await;
    let retry = run(
        &operator,
        update("u3-retry", "http://atlas/hold/image-3.0.img.xz", SHA),
    )
    .await;
    assert_eq!(retry.state, ActionState::Succeeded);
    assert_eq!(phase(&retry), Some(&text(update_action::PHASE_STAGING)));
    let other = run(
        &operator,
        update("u4", "http://atlas/image-4.0.img.xz", &"a".repeat(64)),
    )
    .await;
    assert!(rejected(&other, "update.cancel"), "{other:?}");
    let rollback = run(&operator, node_action("rb1", "update.rollback")).await;
    assert!(rejected(&rollback, "staging"), "{rollback:?}");
    let cancelled = run(&operator, node_action("c1", "update.cancel")).await;
    assert_eq!(cancelled.state, ActionState::Succeeded, "{cancelled:?}");
    assert_eq!(
        phase(&cancelled),
        Some(&text(update_action::PHASE_CANCELLED))
    );
    board
        .wait_value(update_action::KEY_STATE, text("cancelled"))
        .await;
    // The cancelled stage is not applied, even once it could go on.
    std::fs::write(board.run.join("release"), "").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        board
            .calls()
            .iter()
            .filter(|c| c.as_str() == "apply")
            .count(),
        1
    );

    // Nothing to cancel.
    let idle = run(&operator, node_action("c2", "update.cancel")).await;
    assert_eq!(phase(&idle), Some(&text(update_action::PHASE_IDLE)));

    // A staged update whose restart is refused stays staged, with why.
    std::fs::write(board.run.join("refuse-apply"), "").unwrap();
    run(
        &operator,
        update("u5", "http://atlas/image-5.0.img.xz", SHA),
    )
    .await;
    board
        .wait_value(update_action::KEY_STATE, text("staged"))
        .await;
    board
        .wait_value(
            update_action::KEY_ERROR,
            text("the pre-reboot hook stopped the restart; the update is still staged"),
        )
        .await;
    // ...and cancel forgets it.
    let forgot = run(&operator, node_action("c3", "update.cancel")).await;
    assert_eq!(phase(&forgot), Some(&text(update_action::PHASE_CANCELLED)));

    // The writer refuses (exit status 3: a trial boot is running): so does
    // the action.
    let env = board.run.join("fake.env");
    let state = std::fs::read_to_string(&env).unwrap();
    std::fs::write(&env, format!("{state}STATE=trying\n")).unwrap();
    let refused = run(
        &operator,
        update("u6", "http://atlas/image-6.0.img.xz", SHA),
    )
    .await;
    assert!(rejected(&refused, "still on trial"), "{refused:?}");

    board.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rollback_reboot_and_locate() {
    let board = Board::start("actions").await;
    let operator = board.operator();
    board
        .wait_value(update_action::KEY_STATE, text("idle"))
        .await;

    // No previous confirmed slot: the writer refuses, and so does the action.
    let refused = run(&operator, node_action("rb1", "update.rollback")).await;
    assert!(
        rejected(&refused, "no previous confirmed slot"),
        "{refused:?}"
    );
    assert!(!board.calls().iter().any(|c| c == "reboot"));

    // With one: switch, report rebooting, then reboot.
    board.set_previous("0.9");
    let rolled = run(&operator, node_action("rb2", "update.rollback")).await;
    assert_eq!(rolled.state, ActionState::Succeeded, "{rolled:?}");
    assert_eq!(phase(&rolled), Some(&text(update_action::PHASE_REBOOTING)));
    board
        .wait_value(update_action::KEY_STATE, text("rebooting"))
        .await;
    board
        .wait_value(update_action::KEY_VERSION_ACTIVE, text("0.9"))
        .await;
    for _ in 0..200 {
        if board.calls().iter().any(|c| c == "reboot") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        board.calls().contains(&"rollback --no-reboot".to_owned()),
        "{:?}",
        board.calls()
    );
    assert!(
        board.calls().iter().any(|c| c == "reboot"),
        "{:?}",
        board.calls()
    );

    // reboot: reported first.
    let reboot = run(
        &operator,
        node_action("r1", "reboot").with_arg("delay_ms", TypedConfigValue::UInt(10)),
    )
    .await;
    assert_eq!(phase(&reboot), Some(&text(update_action::PHASE_REBOOTING)));
    for _ in 0..200 {
        if board.calls().iter().any(|c| c.starts_with("event reboot")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        board
            .calls()
            .contains(&"event reboot restart requested through Orion source=orion".to_owned()),
        "{:?}",
        board.calls()
    );

    // locate drops the package's request; enabled=false stops it.
    let locate = run(&operator, node_action("l1", "locate")).await;
    assert_eq!(locate.state, ActionState::Succeeded, "{locate:?}");
    assert_eq!(
        std::fs::read_to_string(board.run.join("requests/locate")).unwrap(),
        "10\n"
    );
    let stop = run(
        &operator,
        node_action("l2", "locate").with_arg("enabled", TypedConfigValue::Bool(false)),
    )
    .await;
    assert_eq!(stop.state, ActionState::Succeeded, "{stop:?}");
    assert!(board.calls().iter().any(|c| c == "locate-stop"));

    // clock.set: the agent lists it, sets the time it is sent and logs it as
    // Orion's; nonsense times are refused before anything runs.
    assert_eq!(
        board.value("action.claimed"),
        Some(text(
            "update,update.cancel,update.rollback,reboot,locate,clock.set,usb-route.set,slot.boot-once,slot.set-default,slot.remove,slot.pin"
        ))
    );
    let set = run(
        &operator,
        node_action("c1", "clock.set").with_arg("unix", TypedConfigValue::Int(1_791_590_400)),
    )
    .await;
    assert_eq!(set.state, ActionState::Succeeded, "{set:?}");
    assert!(
        set.output.contains_key("old") && set.output.contains_key("new"),
        "{set:?}"
    );
    assert!(
        board.calls().iter().any(|c| c == "date @1791590400"),
        "{:?}",
        board.calls()
    );
    assert!(
        board.calls().iter().any(
            |c| c.starts_with("event clock.set clock set through Orion old=")
                && c.ends_with(" source=orion")
        ),
        "{:?}",
        board.calls()
    );
    let iso = run(
        &operator,
        node_action("c2", "clock.set").with_arg("time", text("2026-10-10T00:00:01Z")),
    )
    .await;
    assert_eq!(iso.state, ActionState::Succeeded, "{iso:?}");
    assert!(
        board.calls().iter().any(|c| c == "date @1791590401"),
        "{:?}",
        board.calls()
    );
    // usb-route.set: the package's usb-route, on or off; `on` is required.
    let off = run(
        &operator,
        node_action("u1", "usb-route.set").with_arg("on", TypedConfigValue::Bool(false)),
    )
    .await;
    assert_eq!(off.state, ActionState::Succeeded, "{off:?}");
    assert!(
        board.calls().iter().any(|c| c == "usb-route set off"),
        "{:?}",
        board.calls()
    );
    let bare = run(&operator, node_action("u2", "usb-route.set")).await;
    assert!(rejected(&bare, "`on` (bool) is required"), "{bare:?}");
    let wrong = run(
        &operator,
        node_action("c3", "clock.set").with_arg("unix", TypedConfigValue::Int(0)),
    )
    .await;
    assert!(rejected(&wrong, "not a plausible time"), "{wrong:?}");
    assert!(
        !board.calls().iter().any(|c| c == "date @0"),
        "{:?}",
        board.calls()
    );
    assert!(
        !board
            .calls()
            .iter()
            .any(|c| c.starts_with("source-missing")),
        "{:?}",
        board.calls()
    );

    board.stop().await;
}
