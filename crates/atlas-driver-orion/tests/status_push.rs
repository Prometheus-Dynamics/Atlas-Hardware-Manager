//! Orion's status push channel against a real orion-node: an agent claims
//! the node's update action on its IPC and publishes the `update.*` keys,
//! as board-agent does, and Atlas, not enrolled (the node opens status to
//! read), watches them over `orion+tcp`: a hello, then an event for each
//! new event seq and an update for each new update state.

#![cfg(unix)]

#[allow(dead_code)]
mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_driver::{CapabilitySource, PushSink, StatusCapability, StatusPush};
use atlas_driver_orion::{LiveStreams, OperatorIdentity, OrionDirectory};
use orion_client::{ActionReporter, LocalNodeRuntime, LocalProviderService};
use orion_control_plane::{ProviderRecord, TypedConfigValue};
use orion_core::ProviderId;
use orion_node::{NodeApp, NodeConfig, NodeId, OperatorOpenRead};
use support::{FakeOrion, NODE, raze, text};

struct Board {
    dir: PathBuf,
    address: String,
    app: NodeApp,
    _servers: Vec<Box<dyn std::any::Any + Send>>,
}

/// The board's node, open to read its status, on a TCP port of its own.
async fn board(label: &str) -> Board {
    let dir = std::env::temp_dir().join(format!("atlas-push-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("control.sock");
    let stream_socket = dir.join("control-stream.sock");
    let config = NodeConfig::for_local_node(NodeId::new(NODE))
        .with_ipc_socket_path(socket.clone())
        .with_runtime_tuning_mut(|mut tuning| {
            tuning.actions = tuning.actions.with_operator_open_read(OperatorOpenRead {
                status: true,
                streams: true,
                state: false,
            });
            tuning
        });
    let app = NodeApp::try_new(config).unwrap();
    let (_, unary) = app.start_ipc_server_graceful(&socket).await.unwrap();
    let (_, streams) = app
        .start_ipc_stream_server_graceful(&stream_socket)
        .await
        .unwrap();
    let (addr, peer) = app
        .start_peer_tcp_server("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    Board {
        address: format!("orion+tcp://{addr}"),
        dir,
        app,
        _servers: vec![Box::new(unary), Box::new(streams), Box::new(peer)],
    }
}

/// An agent on the board's IPC that claims `update`, so it may publish the
/// `update.*` keys.
async fn agent(board: &Board) -> (ActionReporter, impl Sized + use<>) {
    let runtime = LocalNodeRuntime::new(
        board.dir.join("control.sock"),
        board.dir.join("control-stream.sock"),
    );
    let service = LocalProviderService::new(
        runtime,
        "board-agent",
        ProviderRecord::builder(ProviderId::new("board-agent"), NodeId::new("local")).build(),
    );
    let watch = service.claim_node_actions(["update"]).await.unwrap();
    (watch.reporter(), (watch, service))
}

async fn publish(reporter: &ActionReporter, keys: &[(&str, TypedConfigValue)]) {
    reporter
        .publish_status(
            keys.iter()
                .map(|(key, value)| reporter.node_status_entry(*key, value.clone())),
        )
        .await
        .unwrap();
}

/// Orion's status capability for the fake Raze at `address`, its node
/// reached with `live`.
async fn status_for(address: &str) -> (Arc<dyn StatusCapability>, atlas_driver::Identity) {
    let directory = OrionDirectory::new(Arc::new(FakeOrion::new()), None);
    directory.set_live_streams(Arc::new(LiveStreams::new(
        OperatorIdentity::generate("atlas-test").unwrap(),
        None,
    )));
    directory.refresh().await.unwrap();
    let mut device = raze(Some("e5226d57"));
    device.address = address.to_owned();
    let status = directory.capabilities_for(&device).status.unwrap();
    (status, device)
}

fn collect() -> (PushSink, Arc<Mutex<Vec<StatusPush>>>) {
    let pushes = Arc::new(Mutex::new(Vec::new()));
    let seen = pushes.clone();
    let sink: PushSink = Arc::new(move |push| seen.lock().unwrap().push(push));
    (sink, pushes)
}

async fn until(pushes: &Mutex<Vec<StatusPush>>, count: usize) -> Vec<StatusPush> {
    for _ in 0..100 {
        if pushes.lock().unwrap().len() >= count {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    pushes.lock().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_agents_keys_are_pushed() {
    let board = board("keys").await;
    let (reporter, _agent) = agent(&board).await;
    publish(
        &reporter,
        &[
            ("update.boot_id", text("b-1")),
            ("update.state", text("idle")),
            ("update.event_seq", TypedConfigValue::UInt(4)),
            ("update.event_kind", text("boot")),
            ("update.event_t", TypedConfigValue::Int(1_760_000_000)),
        ],
    )
    .await;

    let (status, device) = status_for(&board.address).await;
    let (sink, pushes) = collect();
    let watching = tokio::spawn(async move { status.watch(&device, sink).await });
    assert_eq!(
        until(&pushes, 1).await,
        vec![StatusPush::Hello {
            boot_id: "b-1".into(),
            newest_seq: Some(4),
        }]
    );

    publish(
        &reporter,
        &[
            ("update.event_seq", TypedConfigValue::UInt(5)),
            ("update.event_kind", text("update.staging")),
            ("update.event_t", TypedConfigValue::Int(1_760_000_090)),
            ("update.state", text("staging")),
        ],
    )
    .await;
    let seen = until(&pushes, 3).await;
    let StatusPush::Event(event) = &seen[1] else {
        panic!("an event: {seen:?}");
    };
    assert_eq!(
        (
            event.seq,
            event.kind.as_str(),
            event.t,
            event.boot_id.as_str()
        ),
        (Some(5), "update.staging", 1_760_000_090, "b-1")
    );
    assert_eq!(
        seen[2],
        StatusPush::Update {
            state: "staging".into()
        }
    );

    // Progress alone pushes nothing: the status read covers it.
    publish(
        &reporter,
        &[("update.progress", TypedConfigValue::UInt(300))],
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(pushes.lock().unwrap().len(), 3);
    assert!(!watching.is_finished(), "the channel stays open");

    watching.abort();
    drop(board.app);
    let _ = std::fs::remove_dir_all(&board.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_node_without_an_agent_is_polled() {
    let board = board("no-agent").await;
    let (status, device) = status_for(&board.address).await;
    let (sink, pushes) = collect();
    let result = tokio::time::timeout(Duration::from_secs(5), status.watch(&device, sink))
        .await
        .expect("answers at once");
    assert!(result.is_err(), "{result:?}");
    assert!(pushes.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&board.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_node_is_polled() {
    let (status, device) = status_for("orion+tcp://127.0.0.1:1").await;
    let (sink, _) = collect();
    assert!(status.watch(&device, sink).await.is_err());

    // Without a way to reach the board's node there is no channel at all.
    let directory = OrionDirectory::new(Arc::new(FakeOrion::new()), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let status = directory.capabilities_for(&device).status.unwrap();
    let (sink, _) = collect();
    assert!(!status.watch(&device, sink).await.unwrap());
}
