//! board-agent against a real orion-node (in-process, on IPC sockets), with
//! a fake writer (tests/fake-writer.sh) standing in for
//! /usr/lib/board/update: the contract of Orion's docs/device-agent.md.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use board_agent::Config;
use orion_client::LocalControlPlaneClient;
use orion_control_plane::{
    ActionRequest, ActionResult, ActionState, ActionTarget, StatusQuery, StatusSubject,
    TypedConfigValue, update_action,
};
use orion_node::{NodeApp, NodeConfig, NodeId};

const SHA: &str = "4f0d6a2c9b8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a392817065f4e";

struct Board {
    dir: PathBuf,
    run: PathBuf,
    app: NodeApp,
    socket: PathBuf,
    servers: tokio::task::JoinHandle<()>,
    agent: tokio::task::JoinHandle<()>,
}

impl Board {
    async fn start(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("board-agent-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = dir.join("run");
        std::fs::create_dir_all(run.join("requests")).unwrap();
        let writer = dir.join("update");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-writer.sh"),
            &writer,
        )
        .unwrap();
        std::fs::set_permissions(&writer, std::fs::Permissions::from_mode(0o755)).unwrap();
        // "Reboot" and "stop locating" only leave a trace.
        let record = dir.join("record");
        std::fs::write(
            &record,
            format!(
                "#!/bin/sh\n[ \"$1\" != event ] || set -- \"$@\" \"source=$BOARD_EVENT_SOURCE\"\necho \"$*\" >> {}/calls\n",
                run.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("boot_id"), "boot-1\n").unwrap();

        let socket = dir.join("control.sock");
        let stream = dir.join("control-stream.sock");
        let app = NodeApp::try_new(
            NodeConfig::for_local_node(NodeId::new("raze-1")).with_ipc_socket_path(socket.clone()),
        )
        .unwrap();
        let (_, unary) = app.start_ipc_server_graceful(&socket).await.unwrap();
        let (_, stream_server) = app.start_ipc_stream_server_graceful(&stream).await.unwrap();
        let servers = tokio::spawn(async move {
            let _servers = (unary, stream_server);
            std::future::pending::<()>().await;
        });
        let config = Config {
            control_socket: socket.clone(),
            stream_socket: stream,
            writer,
            run_dir: run.clone(),
            boot_id_file: dir.join("boot_id"),
            reboot_command: vec![record.display().to_string(), "reboot".into()],
            locate_stop_command: vec![record.display().to_string(), "locate-stop".into()],
            event_command: vec![record.display().to_string(), "event".into()],
            poll: Duration::from_millis(50),
            republish: Duration::from_millis(500),
            retry: Duration::from_millis(20),
        };
        let agent = tokio::spawn(async move {
            let _ = board_agent::run(config).await;
        });
        Self {
            dir,
            run,
            app,
            socket,
            servers,
            agent,
        }
    }

    fn operator(&self) -> LocalControlPlaneClient {
        LocalControlPlaneClient::connect_at(&self.socket, "operator").unwrap()
    }

    fn value(&self, key: &str) -> Option<TypedConfigValue> {
        self.app
            .query_status(
                &StatusQuery::subject(StatusSubject::Node(NodeId::new("raze-1")))
                    .with_key_prefix(key),
            )
            .into_iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value)
    }

    async fn wait_value(&self, key: &str, expected: TypedConfigValue) {
        for _ in 0..500 {
            if self.value(key).as_ref() == Some(&expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "{key} never became {expected:?} (now {:?}); writer calls: {:?}",
            self.value(key),
            self.calls()
        );
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.run.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn set_previous(&self, version: &str) {
        let env = self.run.join("fake.env");
        let text = std::fs::read_to_string(&env).unwrap_or_default();
        std::fs::write(env, format!("{text}PREVIOUS='{version}'\n")).unwrap();
    }

    async fn stop(self) {
        self.agent.abort();
        self.servers.abort();
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

fn text(value: &str) -> TypedConfigValue {
    TypedConfigValue::String(value.into())
}

fn node_action(id: &str, name: &str) -> ActionRequest {
    ActionRequest::new(id, ActionTarget::Node(NodeId::new("raze-1")), name)
}

fn update(id: &str, url: &str, sha256: &str) -> ActionRequest {
    node_action(id, "update")
        .with_arg(update_action::ARG_IMAGE_URL, text(url))
        .with_arg(update_action::ARG_SHA256, text(sha256))
        .with_arg(update_action::ARG_SIZE, TypedConfigValue::UInt(1 << 20))
}

async fn run(operator: &LocalControlPlaneClient, request: ActionRequest) -> ActionResult {
    // The agent has claimed its actions once it published update.state,
    // which each test waits for first.
    let id = request.action_id.clone();
    operator.run_action(request).await.unwrap();
    operator
        .wait_for_action(&id, Duration::from_millis(10), Duration::from_secs(20))
        .await
        .unwrap()
}

fn phase(result: &ActionResult) -> Option<&TypedConfigValue> {
    result.output.get(update_action::OUTPUT_PHASE)
}

fn rejected(result: &ActionResult, needle: &str) -> bool {
    matches!(&result.state, ActionState::Rejected { reason } if reason.contains(needle))
}

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
