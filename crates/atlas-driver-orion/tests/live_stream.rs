//! `LiveStreams` against a real orion-node: a publisher declares
//! board-agent's `board.live` on the node's IPC, and Atlas, not enrolled
//! (the node opens streams to read), follows it over `orion+tcp`: one
//! backfill of rows with the history, then rows as they complete.

#![cfg(unix)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_driver_orion::{LiveFrame, LiveStreams, OperatorIdentity};
use orion_client::StreamPublisher;
use orion_control_plane::{
    HistoryPolicy, SampleChunkBuilder, SeriesDecl, SeriesKind, StatusSubject, StreamDecl, StreamRef,
};
use orion_node::{NodeApp, NodeConfig, NodeId, OperatorOpenRead};

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rows_arrive_backfilled_then_live() {
    let dir = std::env::temp_dir().join(format!("atlas-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("control.sock");
    let stream_socket = dir.join("control-stream.sock");
    let config = NodeConfig::for_local_node(NodeId::new("raze-1"))
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

    let stream = StreamRef {
        owner: StatusSubject::Node(NodeId::new("raze-1")),
        id: "board.live".into(),
    };
    let mut publisher = StreamPublisher::connect_at(&stream_socket, "board-agent.live")
        .await
        .unwrap();
    let gauge = |name: &str, unit: &str| {
        SeriesDecl::new(name, SeriesKind::F32, 100_000)
            .with_unit(unit)
            .with_history(HistoryPolicy::full(300_000))
    };
    publisher
        .declare(StreamDecl {
            owner: stream.owner.clone(),
            id: stream.id.clone(),
            series: vec![
                gauge("cpu", "%"),
                gauge("cpu.core0", "%"),
                gauge("cpu-thermal.temp", "°C"),
                gauge("rp1-adc.temp", "°C"),
                SeriesDecl::new("uptime", SeriesKind::F64, 1_000_000)
                    .with_history(HistoryPolicy::full(300_000)),
            ],
        })
        .await
        .unwrap();
    // Ten seconds of history at 10 Hz.
    let start = now_us() - 10_000_000;
    let samples = |from: u64, to: u64| {
        let mut chunks = [
            SampleChunkBuilder::new(0, SeriesKind::F32),
            SampleChunkBuilder::new(1, SeriesKind::F32),
            SampleChunkBuilder::new(2, SeriesKind::F32),
            SampleChunkBuilder::new(3, SeriesKind::F32),
            SampleChunkBuilder::new(4, SeriesKind::F64),
        ];
        for n in from..to {
            let t = start + n * 100_000;
            chunks[0].push(t, 40.0);
            chunks[1].push(t, 20.0);
            chunks[2].push(t, 55.0);
            chunks[3].push(t, 60.0);
            if n % 10 == 0 {
                chunks[4].push(t, 1_000.0 + n as f64 / 10.0);
            }
        }
        chunks
            .iter_mut()
            .map(SampleChunkBuilder::take)
            .collect::<Vec<_>>()
    };
    publisher.publish(&stream, samples(0, 100)).await.unwrap();

    let frames = Arc::new(Mutex::new(Vec::new()));
    let seen = frames.clone();
    let identity = OperatorIdentity::generate("atlas-test").unwrap();
    let live = LiveStreams::new(identity, Some(dir.join("pins")));
    let follow = tokio::spawn(async move {
        live.follow(&format!("orion+tcp://{addr}"), move |frame| {
            seen.lock().unwrap().push(frame);
            true
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    publisher.publish(&stream, samples(100, 120)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let frames = frames.lock().unwrap().clone();
    let LiveFrame::Rows {
        backfill: true,
        rows,
        ..
    } = &frames[0]
    else {
        panic!("a backfill first: {frames:?}");
    };
    // 10 s of 500 ms rows, the newest still open.
    assert!((18..=21).contains(&rows.len()), "{}", rows.len());
    let row = &rows[2];
    assert_eq!(row.values["cpu"], 40.0);
    assert_eq!(row.values["cpu.core.0"], 20.0);
    assert_eq!(row.values["temp"], 60.0, "the hottest zone");
    let now = now_us() as f64 / 1000.0;
    assert!(
        (now - rows.last().unwrap().t) < 2_000.0,
        "on this computer's clock"
    );
    let LiveFrame::Rows {
        backfill: false,
        rows: more,
        latest,
    } = frames.last().unwrap()
    else {
        panic!("then live rows: {frames:?}");
    };
    assert!(!more.is_empty());
    let cpu = latest.iter().find(|m| m.id == "cpu").unwrap();
    assert_eq!((cpu.value, cpu.unit.as_deref()), (40.0, Some("%")));
    let temp = latest.iter().find(|m| m.id == "temp").unwrap();
    assert_eq!(temp.value, 60.0);
    assert!(
        latest
            .iter()
            .any(|m| m.id == "temp.cpu-thermal" && m.value == 55.0)
    );
    assert!(dir.join("pins/raze-1").exists(), "the node's key is pinned");

    follow.abort();
    drop((unary, streams, peer));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_board_without_a_listener_is_an_error_at_once() {
    let identity = OperatorIdentity::generate("atlas-test").unwrap();
    let live = LiveStreams::new(identity, None);
    let started = std::time::Instant::now();
    let result = live.follow("orion+tcp://127.0.0.1:1", |_| true).await;
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}
