//! board-agent's slot actions and published keys against a real orion-node
//! (in-process, on IPC sockets), with the fake writer (tests/fake-writer.sh).

#![cfg(target_os = "linux")]

mod common;

use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slot_actions_run_the_slots_writer_and_slots_are_published() {
    let board = Board::start("slots").await;
    let operator = board.operator();
    board
        .wait_value(update_action::KEY_STATE, text("idle"))
        .await;
    // An A/B board refuses them before the writer runs.
    let refused = run(
        &operator,
        node_action("s0", "slot.set-default").with_arg("id", text("pv-b")),
    )
    .await;
    assert!(rejected(&refused, "no slots"), "{refused:?}");
    assert!(!board.calls().iter().any(|c| c.starts_with("set-default")));

    // Now a slots board: its writer's update.json lists the slots.
    std::fs::write(board.run.join("slots-layout"), "").unwrap();
    let status = std::process::Command::new(board.dir.join("update"))
        .arg("status")
        .env("BOARD_RUN_DIR", &board.run)
        .env("BOARD_EVENT_SOURCE", "orion")
        .output()
        .unwrap();
    assert!(status.status.success());
    board
        .wait_value(board_agent::KEY_LAYOUT, text("slots"))
        .await;
    let pin = run(
        &operator,
        node_action("s1", "slot.pin").with_arg("id", text("pv-b")),
    )
    .await;
    assert_eq!(pin.state, ActionState::Succeeded, "{pin:?}");
    assert!(
        board.calls().iter().any(|c| c == "pin pv-b"),
        "{:?}",
        board.calls()
    );
    assert_eq!(
        board.value(board_agent::KEY_SLOT_DEFAULT),
        Some(text("running"))
    );
    assert_eq!(
        board.value(board_agent::KEY_FREE_BYTES),
        Some(TypedConfigValue::UInt(5_000_000_000))
    );
    let Some(TypedConfigValue::String(slots)) = board.value(board_agent::KEY_SLOTS) else {
        panic!("no update.slots");
    };
    let slots: serde_json::Value = serde_json::from_str(&slots).unwrap();
    assert_eq!(slots[1]["id"], "pv-b");

    let unpin = run(
        &operator,
        node_action("s2", "slot.pin")
            .with_arg("id", text("pv-b"))
            .with_arg("pinned", TypedConfigValue::Bool(false)),
    )
    .await;
    assert_eq!(unpin.state, ActionState::Succeeded, "{unpin:?}");
    assert!(board.calls().iter().any(|c| c == "unpin pv-b"));

    // The writer's refusal is the action's.
    let remove = run(
        &operator,
        node_action("s3", "slot.remove").with_arg("id", text("running")),
    )
    .await;
    assert!(rejected(&remove, "running slot"), "{remove:?}");
    let missing = run(&operator, node_action("s4", "slot.remove")).await;
    assert!(rejected(&missing, "`id`"), "{missing:?}");

    let default = run(
        &operator,
        node_action("s5", "slot.set-default").with_arg("id", text("pv-b")),
    )
    .await;
    assert_eq!(default.state, ActionState::Succeeded, "{default:?}");
    assert!(board.calls().iter().any(|c| c == "set-default pv-b"));

    // Boot once answers before the restart.
    let once = run(
        &operator,
        node_action("s6", "slot.boot-once").with_arg("id", text("pv-b")),
    )
    .await;
    assert_eq!(once.state, ActionState::Succeeded, "{once:?}");
    assert_eq!(phase(&once), Some(&text(update_action::PHASE_REBOOTING)));
    for _ in 0..100 {
        if board.calls().iter().any(|c| c == "boot-once pv-b") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        board.calls().iter().any(|c| c == "boot-once pv-b"),
        "{:?}",
        board.calls()
    );

    // Install only: staged with --pin, no apply after.
    std::fs::write(board.run.join("fake.env"), "STATE=idle\n").unwrap();
    let install = run(
        &operator,
        update(
            "s7",
            "http://example.invalid/image-3.0.img.xz",
            &"ab".repeat(32),
        )
        .with_arg("apply", TypedConfigValue::Bool(false))
        .with_arg("pin", TypedConfigValue::Bool(true)),
    )
    .await;
    assert_eq!(install.state, ActionState::Succeeded, "{install:?}");
    board
        .wait_value(update_action::KEY_STATE, text("staged"))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let calls = board.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("stage-url") && c.ends_with("--pin")),
        "{calls:?}"
    );
    assert!(!calls.iter().any(|c| c == "apply"), "{calls:?}");

    // A long list (a real board's): one slot per key, `update.slots` empty,
    // and the rest of the keys still published.
    std::fs::write(board.run.join("slots-many"), "").unwrap();
    let status = std::process::Command::new(board.dir.join("update"))
        .arg("status")
        .env("BOARD_RUN_DIR", &board.run)
        .output()
        .unwrap();
    assert!(status.status.success());
    board
        .wait_value(board_agent::KEY_SLOT_COUNT, TypedConfigValue::UInt(15))
        .await;
    assert_eq!(board.value(board_agent::KEY_SLOTS), Some(text("")));
    let Some(TypedConfigValue::String(last)) =
        board.value(&format!("{}14", board_agent::KEY_SLOT_PREFIX))
    else {
        panic!("no update.slots.14");
    };
    assert!(last.contains("dev.13-styx"), "{last}");
    assert!(board.value("action.claimed").is_some());
    board.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nulls_who_started_it_and_the_last_event_are_published() {
    let board = Board::start("keys").await;
    board
        .wait_value(update_action::KEY_STATE, text("idle"))
        .await;
    // As the writer prints it: an empty value is null.
    std::fs::write(
        board.run.join("update.json"),
        r#"{"state":"confirmed","slot_active":"B","slot_staged":null,"version_active":"2.0","version_staged":null,"progress":0,"error":null,"version_previous":"1.0","started_by":"local"}"#,
    )
    .unwrap();
    board
        .wait_value(update_action::KEY_STATE, text("confirmed"))
        .await;
    assert_eq!(
        board.value(board_agent::KEY_STARTED_BY),
        Some(text("local"))
    );
    assert_eq!(
        board.value(board_agent::KEY_VERSION_PREVIOUS),
        Some(text("1.0"))
    );
    assert_eq!(board.value(update_action::KEY_ERROR), Some(text("")));

    std::fs::create_dir_all(board.dir.join("data")).unwrap();
    std::fs::write(
        board.dir.join("data/events.jsonl"),
        "{\"t\":100,\"boot_id\":\"boot-1\",\"seq\":7,\"uptime_s\":3,\"kind\":\"update.confirmed\",\"source\":\"local\",\"message\":\"kept\",\"data\":{}}\n",
    )
    .unwrap();
    board
        .wait_value(board_agent::KEY_EVENT_SEQ, TypedConfigValue::UInt(7))
        .await;
    assert_eq!(
        board.value(board_agent::KEY_EVENT_KIND),
        Some(text("update.confirmed"))
    );
    assert_eq!(
        board.value(board_agent::KEY_EVENT_T),
        Some(TypedConfigValue::Int(100))
    );
}
