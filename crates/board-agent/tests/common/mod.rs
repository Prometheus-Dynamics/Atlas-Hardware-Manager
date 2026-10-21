//! What the orion-node tests share: a board (orion-node in-process, the
//! agent, a fake writer) and helpers for its actions.
#![allow(dead_code)]

pub use std::os::unix::fs::PermissionsExt;
pub use std::path::{Path, PathBuf};
pub use std::time::Duration;

pub use board_agent::Config;
pub use orion_client::LocalControlPlaneClient;
pub use orion_control_plane::{
    ActionRequest, ActionResult, ActionState, ActionTarget, StatusQuery, StatusSubject,
    TypedConfigValue, update_action,
};
pub use orion_node::{NodeApp, NodeConfig, NodeId};

pub const SHA: &str = "4f0d6a2c9b8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a392817065f4e";

pub struct Board {
    pub dir: PathBuf,
    pub run: PathBuf,
    pub app: NodeApp,
    pub socket: PathBuf,
    pub servers: tokio::task::JoinHandle<()>,
    pub agent: tokio::task::JoinHandle<()>,
}

impl Board {
    pub async fn start(label: &str) -> Self {
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
            data_dir: dir.join("data"),
            boot_id_file: dir.join("boot_id"),
            reboot_command: vec![record.display().to_string(), "reboot".into()],
            locate_stop_command: vec![record.display().to_string(), "locate-stop".into()],
            event_command: vec![record.display().to_string(), "event".into()],
            set_clock_command: vec![record.display().to_string(), "date".into()],
            usb_route_command: vec![record.display().to_string(), "usb-route".into()],
            poll: Duration::from_millis(50),
            republish: Duration::from_millis(500),
            retry: Duration::from_millis(20),
            ..Config::default()
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

    pub fn operator(&self) -> LocalControlPlaneClient {
        LocalControlPlaneClient::connect_at(&self.socket, "operator").unwrap()
    }

    pub fn value(&self, key: &str) -> Option<TypedConfigValue> {
        self.app
            .query_status(
                &StatusQuery::subject(StatusSubject::Node(NodeId::new("raze-1")))
                    .with_key_prefix(key),
            )
            .into_iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value)
    }

    pub async fn wait_value(&self, key: &str, expected: TypedConfigValue) {
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

    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.run.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub fn set_previous(&self, version: &str) {
        let env = self.run.join("fake.env");
        let text = std::fs::read_to_string(&env).unwrap_or_default();
        std::fs::write(env, format!("{text}PREVIOUS='{version}'\n")).unwrap();
    }

    pub async fn stop(self) {
        self.agent.abort();
        self.servers.abort();
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

pub fn text(value: &str) -> TypedConfigValue {
    TypedConfigValue::String(value.into())
}

pub fn node_action(id: &str, name: &str) -> ActionRequest {
    ActionRequest::new(id, ActionTarget::Node(NodeId::new("raze-1")), name)
}

pub fn update(id: &str, url: &str, sha256: &str) -> ActionRequest {
    node_action(id, "update")
        .with_arg(update_action::ARG_IMAGE_URL, text(url))
        .with_arg(update_action::ARG_SHA256, text(sha256))
        .with_arg(update_action::ARG_SIZE, TypedConfigValue::UInt(1 << 20))
}

pub async fn run(operator: &LocalControlPlaneClient, request: ActionRequest) -> ActionResult {
    // The agent has claimed its actions once it published update.state,
    // which each test waits for first.
    let id = request.action_id.clone();
    operator.run_action(request).await.unwrap();
    operator
        .wait_for_action(&id, Duration::from_millis(10), Duration::from_secs(20))
        .await
        .unwrap()
}

pub fn phase(result: &ActionResult) -> Option<&TypedConfigValue> {
    result.output.get(update_action::OUTPUT_PHASE)
}

pub fn rejected(result: &ActionResult, needle: &str) -> bool {
    matches!(&result.state, ActionState::Rejected { reason } if reason.contains(needle))
}
