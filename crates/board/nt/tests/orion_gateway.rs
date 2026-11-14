//! The proof: a viewer against Orion's NT4 gateway, running in process as Orion's own gateway
//! test runs it. A board program (an Orion client) publishes a tank level and reads what the
//! gateway accepts from NT4 clients. The viewer sees the mapped topics with their types and
//! live values, writes a setpoint that reaches the board, has a write of the wrong type refused
//! before it is sent, and comes back by itself when the gateway goes away and returns.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use atlas_nt::{Error, Update, Value, Viewer, ViewerConfig};
use orion::topics::{FieldId, Next, OnSubscribe, SampleTime, TopicDecl};
use orion::{Address, Orion, fields};
use orion_comm_app::{CommCore, CommSettings};
use orion_config::Source;
use tokio::time::timeout;

/// The longest any one wait may take. A wait that works ends in milliseconds; this only bounds a
/// wait that is stuck, so it is generous: a loaded machine (load average 150-200) slows the
/// board's and the viewer's threads, and a 15 s bound failed a passing test there.
const DEADLINE: Duration = Duration::from_secs(60);

static RUNS: AtomicU32 = AtomicU32::new(0);

/// A port that was free a moment ago. The gateway binds it next, and a restart rebinds it.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("its address")
        .port()
}

/// A comm core with the NT4 gateway, in a thread of its own, until stopped.
struct Board {
    socket: PathBuf,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    dir: PathBuf,
}

impl Board {
    fn start(port: u16) -> Self {
        let run = RUNS.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("atlas-nt-gw-{}-{run}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let text = format!(
            "node.name = board\n\
             socket = {0}/orion.sock\n\
             state.dir = {0}/state\n\
             nt4.listen = 127.0.0.1:{port}\n\
             nt4.period_ms = 10\n\
             nt4.publish = tank.level:litres=double@/Tank/level, tank.level:full=boolean@/Tank/full\n\
             nt4.accept = panel.setpoint:value=double@/Panel/setpoint, panel.setpoint:mode=string@/Panel/mode\n",
            dir.display()
        );
        let mut source = Source::parse(&text, Vec::new()).expect("source");
        let settings = CommSettings::from_source(&mut source).expect("settings");
        assert!(source.warnings().is_empty(), "{:?}", source.warnings());
        let (ready, started) = mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            tokio::task::LocalSet::new().block_on(&runtime, async move {
                let core = CommCore::start(settings)
                    .await
                    .expect("the comm core starts");
                let nt4 = core.nt4().expect("the gateway runs").local_addr();
                ready.send((core.socket().to_owned(), nt4)).expect("ready");
                let _ = stopped.await;
            });
        });
        let (socket, nt4) = started.recv().expect("the board starts");
        assert_eq!(nt4.port(), port, "the gateway listens where it was asked");
        Self {
            socket,
            stop: Some(stop),
            thread: Some(thread),
            dir,
        }
    }

    /// Stops the comm core and waits for its thread.
    fn stop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The board's programs: a tank that publishes its level, and a panel reader that reports the
/// setpoints the gateway passes on from NT4 clients (`(field, payload)`).
struct Programs {
    stop: std::sync::Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Programs {
    fn start(socket: &Path, setpoints: mpsc::Sender<f64>) -> Self {
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let (socket, flag) = (socket.to_owned(), stop.clone());
        let thread = std::thread::spawn(move || run(&socket, &flag, &setpoints));
        Self {
            stop,
            thread: Some(thread),
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the programs end");
        }
    }
}

impl Drop for Programs {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(socket: &Path, stop: &AtomicBool, setpoints: &mpsc::Sender<f64>) {
    let orion = Orion::connect(&Address::Local(socket.to_owned()), "tank").expect("connect");
    let mut level = orion
        .declare(&TopicDecl {
            name: "tank.level",
            fields: &fields![litres: Float "l", full: Bool],
            history: 8,
            expiry_ms: 0,
            max_payload: 16,
            on_subscribe: OnSubscribe::FromNow,
        })
        .expect("declare");
    let mut panel = orion.subscribe("panel.setpoint").expect("subscribe");
    let mut step = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        step += 1;
        let time = SampleTime::from_nanos(step);
        let litres = step as f64 * 0.5;
        level
            .publish(FieldId(0), time, &litres.to_le_bytes())
            .expect("publish");
        level
            .publish(FieldId(1), time, &[u8::from(litres > 5.0)])
            .expect("publish");
        while let Next::Sample(sample) = panel.next() {
            if sample.field.0 == 0 {
                let bytes: [u8; 8] = sample.payload.try_into().expect("a double");
                let _ = setpoints.send(f64::from_le_bytes(bytes));
            }
        }
        orion.pump(Duration::from_millis(5)).expect("pump");
    }
}

/// Pumps the viewer until the table it keeps satisfies `ready`. Waits on the state, not on one
/// event: an update consumed while waiting for one topic is still in the table, so waiting for
/// `/Tank/full` after `/Tank/level` cannot miss it when the gateway announced it first.
async fn until_table(viewer: &mut Viewer, mut ready: impl FnMut(&Viewer) -> bool) {
    timeout(DEADLINE, async {
        while !ready(viewer) {
            viewer.next().await.expect("the viewer runs");
        }
    })
    .await
    .expect("the viewer reached the state it waits for");
}

async fn until<T>(viewer: &mut Viewer, mut wanted: impl FnMut(&Update) -> Option<T>) -> T {
    timeout(DEADLINE, async {
        loop {
            let update = viewer.next().await.expect("the viewer runs");
            if let Some(found) = wanted(&update) {
                return found;
            }
        }
    })
    .await
    .expect("the update came")
}

/// The next setpoint the board's programs read, waiting without holding up the viewer.
async fn on_the_board(setpoints: &mpsc::Receiver<f64>) -> f64 {
    timeout(DEADLINE, async {
        loop {
            if let Ok(value) = setpoints.try_recv() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a setpoint on the board")
}

fn viewer_config(port: u16) -> ViewerConfig {
    ViewerConfig {
        host: "127.0.0.1".into(),
        port,
        name: "robot".into(),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
fn a_viewer_sees_and_writes_through_orion_gateway_and_comes_back() {
    let port = free_port();
    let mut board = Board::start(port);
    let (setpoint_tx, setpoints) = mpsc::channel();
    let mut programs = Programs::start(&board.socket, setpoint_tx.clone());

    runtime().block_on(async {
        let mut viewer = Viewer::connect(viewer_config(port)).await.expect("viewer");
        until(&mut viewer, |u| {
            matches!(
                u,
                Update::Connection {
                    connected: true,
                    ..
                }
            )
            .then_some(())
        })
        .await;

        // The mapped fields are topics, under the NT4 names and types the mapping gives.
        // The two announces may come in either order: both are waited for in the table.
        until_table(&mut viewer, |v| {
            v.topic("/Tank/level").is_some() && v.topic("/Tank/full").is_some()
        })
        .await;
        assert_eq!(
            viewer.topic("/Tank/level").map(|t| t.type_name.as_str()),
            Some("double")
        );
        assert_eq!(
            viewer.topic("/Tank/full").map(|t| t.type_name.as_str()),
            Some("boolean")
        );

        // Their values follow the board's: the level rises half a litre at a time.
        let level_of = |u: &Update| match u {
            Update::Value { name, sample } if name == "/Tank/level" => match sample.value {
                Value::Double(litres) => Some(litres),
                _ => None,
            },
            _ => None,
        };
        let first = until(&mut viewer, |u| level_of(u)).await;
        let later = until(&mut viewer, |u| level_of(u).filter(|l| *l > first + 2.0)).await;
        assert_eq!(later % 0.5, 0.0, "{later}");
        assert!(
            viewer
                .topic("/Tank/level")
                .and_then(|t| t.sample.as_ref())
                .is_some()
        );

        // A setpoint the gateway accepts reaches the board, as a double.
        viewer
            .publish_value("/Panel/setpoint", "double", Value::Double(7.5))
            .await
            .expect("the write is sent");
        assert_eq!(on_the_board(&setpoints).await, 7.5);

        // A write of the wrong type is refused before anything is sent: the board's next
        // setpoint is the one that fits.
        let refused = viewer
            .publish_value("/Panel/setpoint", "double", Value::Int(3))
            .await;
        assert!(
            matches!(refused, Err(Error::TypeMismatch { .. })),
            "{refused:?}"
        );
        viewer
            .publish_value("/Panel/setpoint", "double", Value::Double(8.5))
            .await
            .expect("the write is sent");
        assert_eq!(on_the_board(&setpoints).await, 8.5);

        // Once announced, a topic's own type is the one edits must match.
        let refused = viewer.edit("/Tank/full", Value::Double(1.0)).await;
        assert!(
            matches!(refused, Err(Error::TypeMismatch { .. })),
            "{refused:?}"
        );

        // The gateway goes away: the viewer reports the drop and empties its table.
        programs.stop();
        board.stop();
        until(&mut viewer, |u| {
            matches!(
                u,
                Update::Connection {
                    connected: false,
                    ..
                }
            )
            .then_some(())
        })
        .await;
        assert_eq!(
            viewer.topics().count(),
            0,
            "a dropped link empties the table"
        );

        // It comes back: the viewer reconnects by itself and sees the topics again.
        let board = Board::start(port);
        let (setpoint_tx, setpoints) = mpsc::channel();
        let mut programs = Programs::start(&board.socket, setpoint_tx);
        until(&mut viewer, |u| {
            matches!(
                u,
                Update::Connection {
                    connected: true,
                    ..
                }
            )
            .then_some(())
        })
        .await;
        until_table(&mut viewer, |v| v.topic("/Tank/level").is_some()).await;
        until(&mut viewer, |u| level_of(u)).await;

        // Writes work on the new gateway too: the publication is sent again on connect.
        viewer
            .publish_value("/Panel/setpoint", "double", Value::Double(9.25))
            .await
            .expect("the write is sent");
        // The client sends each publication's last value again on connect, so the new board
        // first reads the last setpoint replayed, and then the new one.
        loop {
            let value = on_the_board(&setpoints).await;
            if value == 9.25 {
                break;
            }
            assert_eq!(value, 8.5, "only the replayed last value may come first");
        }
        programs.stop();
        drop(viewer);
        drop(board);
    });
}
