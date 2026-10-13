//! The live streams (`board_agent::live`) against a real orion-node
//! (in-process, on IPC sockets), sampling this machine's /proc and /sys:
//! a subscriber gets the catalog, a backfill of what is sampled always, and
//! the series it asked for at full rate.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use board_agent::Config;
use board_agent::live::{STREAM_HARDWARE, STREAM_SYS, sampling};
use lemnos_ipc::{
    Axis, ChannelDesc, DeviceClass, DeviceDesc, DeviceStatus, Message, Quantity, RawReading,
    Request, VERSION, decode_request,
};
use orion_client::ControlPlaneEventStream;
use orion_control_plane::{
    Backfill, ClientEventKind, StatusSubject, StreamRef, StreamSubscription,
};
use orion_node::{NodeApp, NodeConfig, NodeId};

struct Node {
    dir: PathBuf,
    stream_socket: PathBuf,
    _servers: Box<dyn std::any::Any + Send>,
}

async fn node(label: &str) -> Node {
    let dir = std::env::temp_dir().join(format!("board-agent-live-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("control.sock");
    let stream_socket = dir.join("control-stream.sock");
    let app = NodeApp::try_new(
        NodeConfig::for_local_node(NodeId::new("board")).with_ipc_socket_path(socket.clone()),
    )
    .unwrap();
    let (_, unary) = app.start_ipc_server_graceful(&socket).await.unwrap();
    let (_, streams) = app
        .start_ipc_stream_server_graceful(&stream_socket)
        .await
        .unwrap();
    Node {
        dir,
        stream_socket,
        _servers: Box::new((app, unary, streams)),
    }
}

fn boot_us() -> u64 {
    let t = nix::time::clock_gettime(nix::time::ClockId::CLOCK_BOOTTIME).unwrap();
    (t.tv_sec() * 1_000_000 + t.tv_nsec() / 1_000) as u64
}

/// A lemnosd with one IMU (two channels): every connection gets Hello and
/// List answered and, once it subscribes, a reading every 10 ms until it
/// goes. Records the subscriptions.
fn fake_lemnosd(path: PathBuf) -> Arc<Mutex<Vec<String>>> {
    let subscribed = Arc::new(Mutex::new(Vec::new()));
    let record = subscribed.clone();
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { return };
            let record = record.clone();
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let read = socket.read(&mut chunk).unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..read]);
                    while let Ok(Some((request, used))) = decode_request(&buf) {
                        buf.drain(..used);
                        let reply = match request {
                            Request::Hello { .. } => Message::Welcome {
                                version: VERSION,
                                board: "raze".into(),
                                client_id: 1,
                            },
                            Request::List => Message::Devices(vec![imu()]),
                            Request::Subscribe {
                                id,
                                device,
                                period_ms,
                            } => {
                                record.lock().unwrap().push(device.clone());
                                socket
                                    .write_all(
                                        &Message::Reply {
                                            id,
                                            result: Ok(f64::from(period_ms)),
                                        }
                                        .encode(),
                                    )
                                    .unwrap();
                                for n in 0.. {
                                    let reading = Message::Reading(RawReading {
                                        device: device.clone(),
                                        timestamp_us: boot_us(),
                                        status: DeviceStatus::Available,
                                        values: vec![120 + n, -9810],
                                    });
                                    if socket.write_all(&reading.encode()).is_err() {
                                        return;
                                    }
                                    std::thread::sleep(Duration::from_millis(10));
                                }
                                return;
                            }
                            _ => continue,
                        };
                        if socket.write_all(&reply.encode()).is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    subscribed
}

fn imu() -> DeviceDesc {
    let channel = |name: &str, axis| ChannelDesc {
        name: name.into(),
        quantity: Quantity::Acceleration,
        axis,
        exponent: -3,
    };
    DeviceDesc {
        id: "imu".into(),
        label: "IMU".into(),
        class: DeviceClass::Imu,
        model: "bmi088".into(),
        status: DeviceStatus::Available,
        reason: String::new(),
        channels: vec![
            channel("acceleration.x", Axis::X),
            channel("acceleration.y", Axis::Y),
        ],
        controls: Vec::new(),
        pixels: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lemnosd_is_read_only_while_someone_watches() {
    let node = node("hardware").await;
    let lemnosd = node.dir.join("lemnosd.sock");
    let subscribed = fake_lemnosd(lemnosd.clone());
    let config = Config {
        stream_socket: node.stream_socket.clone(),
        lemnosd_socket: lemnosd,
        retry: Duration::from_millis(20),
        ..Config::default()
    };
    let live = tokio::spawn(board_agent::live::run(Arc::new(config)));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        subscribed.lock().unwrap().is_empty(),
        "cheap: nobody watches, nothing is read"
    );

    let mut subscriber = ControlPlaneEventStream::connect_at(&node.stream_socket, "atlas")
        .await
        .unwrap();
    subscriber
        .subscribe_stream(
            StreamSubscription::all(StreamRef {
                owner: StatusSubject::Node(NodeId::new("board")),
                id: STREAM_HARDWARE.into(),
            })
            .with_series(["imu.acceleration.x"]),
        )
        .await
        .unwrap();
    let mut names = Vec::new();
    let mut values = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while values.len() < 10 {
            for event in subscriber.next_events().await.unwrap() {
                match event.event {
                    ClientEventKind::StreamCatalog(catalog) => {
                        names = catalog
                            .series
                            .iter()
                            .map(|s| (s.name.clone(), s.unit.clone()))
                            .collect();
                    }
                    ClientEventKind::StreamData(data) if !data.backfill => {
                        for chunk in &data.chunks {
                            let series = usize::from(chunk.series);
                            assert_eq!(
                                names[series].0, "imu.acceleration.x",
                                "only what was asked"
                            );
                            for (t_us, value) in
                                chunk.samples(orion_control_plane::SeriesKind::F32).unwrap()
                            {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_micros() as u64;
                                assert!(
                                    now.abs_diff(t_us) < 2_000_000,
                                    "on the Unix clock: {t_us} vs {now}"
                                );
                                values.push(value);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("readings arrive once watched");
    assert_eq!(
        names,
        [
            ("imu.acceleration.x".to_owned(), Some("m/s²".to_owned())),
            ("imu.acceleration.y".to_owned(), Some("m/s²".to_owned()))
        ]
    );
    assert_eq!(*subscribed.lock().unwrap(), ["imu"]);
    assert!(values.windows(2).all(|w| w[1] > w[0]), "{values:?}");
    assert!(
        (values[0] - 0.12).abs() < 0.1,
        "fixed point decoded: {values:?}"
    );

    live.abort();
    let _ = std::fs::remove_dir_all(&node.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_subscriber_gets_the_backfill_then_full_rate_batches() {
    let dir = std::env::temp_dir().join(format!("board-agent-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("control.sock");
    let stream_socket = dir.join("control-stream.sock");
    let app = NodeApp::try_new(
        NodeConfig::for_local_node(NodeId::new("board")).with_ipc_socket_path(socket.clone()),
    )
    .unwrap();
    let (_, unary) = app.start_ipc_server_graceful(&socket).await.unwrap();
    let (_, streams) = app
        .start_ipc_stream_server_graceful(&stream_socket)
        .await
        .unwrap();

    // Memory only while watched; the rest of /proc and /sys always.
    let config = Config {
        control_socket: socket.clone(),
        stream_socket: stream_socket.clone(),
        lemnosd_socket: dir.join("no-lemnosd.sock"),
        retry: Duration::from_millis(20),
        live_sampling: sampling::Rules {
            board: sampling::Mode::Cheap,
            series: sampling::Rules::parse_series("memory*=subscribed").unwrap(),
        },
        ..Config::default()
    };
    let live = tokio::spawn(board_agent::live::run(Arc::new(config)));
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    let stream = StreamRef {
        owner: StatusSubject::Node(NodeId::new("board")),
        id: STREAM_SYS.into(),
    };
    let mut subscriber = ControlPlaneEventStream::connect_at(&stream_socket, "atlas")
        .await
        .unwrap();
    subscriber
        .subscribe_stream(StreamSubscription::all(stream).with_backfill(Backfill::LastMs(60_000)))
        .await
        .unwrap();

    let mut names = Vec::new();
    let mut backfill: Vec<(String, u32)> = Vec::new();
    let mut live_memory = 0u32;
    let mut batches = 0u32;
    tokio::time::timeout(Duration::from_secs(10), async {
        while live_memory < 3 {
            for event in subscriber.next_events().await.unwrap() {
                match event.event {
                    ClientEventKind::StreamCatalog(catalog) => {
                        names = catalog.series.iter().map(|s| s.name.clone()).collect();
                    }
                    ClientEventKind::StreamData(data) if data.backfill => {
                        backfill = data
                            .chunks
                            .iter()
                            .map(|c| (names[usize::from(c.series)].clone(), c.count))
                            .collect();
                    }
                    ClientEventKind::StreamData(data) => {
                        batches += 1;
                        for chunk in &data.chunks {
                            if names[usize::from(chunk.series)] == "memory" {
                                live_memory += chunk.count;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    })
    .await
    .expect("memory arrives once watched");

    assert!(names.iter().any(|n| n == "cpu"), "{names:?}");
    assert!(names.iter().any(|n| n == "uptime"), "{names:?}");
    let cpu = backfill.iter().find(|(n, _)| n == "cpu").map(|(_, c)| *c);
    assert!(
        cpu.is_some_and(|c| c >= 5),
        "a second of CPU at 10 Hz: {backfill:?}"
    );
    assert!(
        !backfill.iter().any(|(n, _)| n.starts_with("memory")),
        "memory was not watched before: {backfill:?}"
    );
    assert!(batches >= 3);

    live.abort();
    drop((unary, streams));
    let _ = std::fs::remove_dir_all(&dir);
}

/// This process's CPU time (node, publisher and subscriber together), µs.
fn cpu_us() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    let ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    ticks * 1_000_000 / 100
}

/// `cargo test -p board-agent --test live -- --ignored --nocapture`: the CPU
/// the node, the publisher and one subscriber of everything cost together,
/// per sampling mode, idle and watched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a measurement, ~40 s"]
#[allow(clippy::print_stdout)] // the measurement is the output
async fn cpu_per_sampling_mode() {
    for mode in [
        sampling::Mode::Always,
        sampling::Mode::Cheap,
        sampling::Mode::Subscribed,
    ] {
        let node = node(&format!("cpu-{mode:?}")).await;
        let config = Config {
            stream_socket: node.stream_socket.clone(),
            lemnosd_socket: node.dir.join("no-lemnosd.sock"),
            live_sampling: sampling::Rules {
                board: mode,
                series: Vec::new(),
            },
            ..Config::default()
        };
        let live = tokio::spawn(board_agent::live::run(Arc::new(config)));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let start = cpu_us();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let idle = cpu_us() - start;

        let mut subscriber = ControlPlaneEventStream::connect_at(&node.stream_socket, "atlas")
            .await
            .unwrap();
        subscriber
            .subscribe_stream(StreamSubscription::all(StreamRef {
                owner: StatusSubject::Node(NodeId::new("board")),
                id: STREAM_SYS.into(),
            }))
            .await
            .unwrap();
        let start = cpu_us();
        let mut samples = 0u64;
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                for event in subscriber.next_events().await.unwrap() {
                    if let ClientEventKind::StreamData(data) = event.event {
                        samples += data.chunks.iter().map(|c| u64::from(c.count)).sum::<u64>();
                    }
                }
            }
        })
        .await;
        let watched = cpu_us() - start;
        println!(
            "{mode:?}: idle {:.2}% of a core, watched {:.2}% ({samples} samples in 5 s)",
            idle as f64 / 50_000.0,
            watched as f64 / 50_000.0
        );
        live.abort();
        let _ = std::fs::remove_dir_all(&node.dir);
    }
}
