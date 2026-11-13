//! The test rig: a simulated board behind a relay that can be cut, and a board's slots through it.
#![allow(dead_code)]

use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use std::future::Future;

use atlas_actions::ActionError;
use atlas_board_slots::{BoardSlots, SlotError, SlotInstalls};
use atlas_jobs::{Backoff, DeviceHolds, Events, JobTiming};
use atlas_session::{Handle, Pause, Presence, Sessions};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use atlas_slots::SlotList;
use atlas_types::DeviceKey;
use orion::Address;
use tokio::time;

/// The board id of the simulated board: the sim's bundles are built for it.
pub const BOARD: &str = "simulated";
pub const SERIAL: &str = "0123abcd";

/// A pause that only yields, so the monitor thread does not spin while the node is gone.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        thread::yield_now();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A TCP relay in front of the node. `cut` drops every live connection; the session reconnects
/// through the relay on its own.
#[derive(Debug)]
pub struct Relay {
    addr: SocketAddr,
    live: Arc<Mutex<Vec<(TcpStream, TcpStream)>>>,
    refusing: Arc<AtomicBool>,
}

impl Relay {
    pub fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the relay binds");
        let addr = listener.local_addr().expect("the relay has an address");
        let live: Arc<Mutex<Vec<(TcpStream, TcpStream)>>> = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&live);
        let refusing = Arc::new(AtomicBool::new(false));
        let refused = Arc::clone(&refusing);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                // A board that is away: the connection is closed at once, as if nothing answered.
                if refused.load(Ordering::SeqCst) {
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                }
                let Ok(server) = TcpStream::connect(upstream) else {
                    continue;
                };
                let (Ok(client_out), Ok(server_out)) = (client.try_clone(), server.try_clone())
                else {
                    continue;
                };
                let (Ok(client_cut), Ok(server_cut)) = (client.try_clone(), server.try_clone())
                else {
                    continue;
                };
                lock(&kept).push((client_cut, server_cut));
                let (mut from_client, mut to_server) = (client, server);
                let (mut from_server, mut to_client) = (server_out, client_out);
                thread::spawn(move || {
                    let _ = std::io::copy(&mut from_client, &mut to_server);
                    let _ = to_server.shutdown(Shutdown::Both);
                });
                thread::spawn(move || {
                    let _ = std::io::copy(&mut from_server, &mut to_client);
                    let _ = to_client.shutdown(Shutdown::Both);
                });
            }
        });
        Self {
            addr,
            live,
            refusing,
        }
    }

    /// While `away`, new connections are closed at once: the board is not reachable. Lifting it
    /// lets the session connect again, as the same node.
    pub fn away(&self, away: bool) {
        self.refusing.store(away, Ordering::SeqCst);
    }

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Drops every live connection now.
    pub fn cut(&self) {
        for (client, server) in std::mem::take(&mut *lock(&self.live)) {
            let _ = client.shutdown(Shutdown::Both);
            let _ = server.shutdown(Shutdown::Both);
        }
    }
}

/// The session waits at most 2 s for a short call's answer (`DIRECT_WAIT`), and a later answer
/// comes back as `NotFinished` with the run asked to stop. Under load that happens to a call that
/// is correct and only slow, so the tests make the call again: for a call that is safe to repeat
/// (`list`, `set_default`, `set_starred`, `set_keep`). Every answer other than `NotFinished` is
/// returned as it is, so a refusal is still checked as the board's words. Bounded by the sim's
/// [`DEADLINE`].
pub async fn answered<T, Fut>(what: &str, mut call: impl FnMut() -> Fut) -> Result<T, SlotError>
where
    Fut: Future<Output = Result<T, SlotError>>,
{
    time::timeout(DEADLINE, async {
        loop {
            match call().await {
                Err(SlotError::Board(ActionError::NotFinished { .. })) => {
                    time::sleep(Duration::from_millis(20)).await;
                }
                other => return other,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} did not answer within {DEADLINE:?}"))
}

/// The same for a call that must NOT be repeated blindly (start a trial, remove): after a
/// `NotFinished`, the board's list is read. When the call's effect is there, the call is done. When
/// it is not, the call is made again, as `BoardSlots` documents: the caller checks the list first.
pub async fn answered_once<Fut>(
    rig: &Rig,
    what: &str,
    mut call: impl FnMut() -> Fut,
    effect: impl Fn(&SlotList) -> bool,
) -> Result<(), SlotError>
where
    Fut: Future<Output = Result<(), SlotError>>,
{
    time::timeout(DEADLINE, async {
        loop {
            match call().await {
                Err(SlotError::Board(ActionError::NotFinished { .. })) => {
                    let listed = answered("the slot list", || rig.slots.list()).await?;
                    if effect(&listed) {
                        return Ok(());
                    }
                    time::sleep(Duration::from_millis(20)).await;
                }
                other => return other,
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} did not take effect within {DEADLINE:?}"))
}

/// A scratch directory under the system temp dir, kept short for the node's socket path.
pub fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("atl-c5-{}-{name}", std::process::id()))
}

/// A simulated board with Lemnos, its slots, and the slot operations through a session.
pub struct Rig {
    pub node: SimNode,
    pub relay: Relay,
    pub sessions: Arc<Sessions>,
    pub handle: Handle,
    pub slots: BoardSlots,
    pub installs: SlotInstalls,
    pub dir: PathBuf,
}

pub fn kind() -> Kind {
    Kind::WithLemnos(Hello {
        family: BOARD.into(),
        serial: SERIAL.into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    })
}

pub fn device() -> DeviceKey {
    DeviceKey::board(BOARD, SERIAL).expect("a board key")
}

/// Waits until `check` holds, within the generous deadline.
pub async fn until(what: &str, mut check: impl FnMut() -> bool) {
    time::timeout(DEADLINE, async {
        while !check() {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// Waits until the handle's monitor reports the node connected.
pub async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected")
    .expect("the monitor is running");
}

impl Rig {
    pub async fn start(name: &str) -> Self {
        let dir = scratch(name);
        let _ = std::fs::remove_dir_all(&dir);
        let node = SimNode::start_with(&dir.join("node"), &kind(), None).expect("the node starts");
        let upstream =
            SocketAddr::from_str(&format!("127.0.0.1:{}", node.network().port())).unwrap();
        let relay = Relay::start(upstream);
        let address = Address::from_str(&format!("orion+tcp://{}", relay.addr())).unwrap();
        let sessions = Arc::new(Sessions::new(Arc::new(NoPause)));
        let handle = sessions.handle(&address).expect("the node is on");
        connected(&handle).await;
        let slots = BoardSlots::new(handle.clone(), BOARD, true);
        let installs = open_installs(&dir, Arc::clone(&sessions)).await;
        Self {
            node,
            relay,
            sessions,
            handle,
            slots,
            installs,
            dir,
        }
    }

    /// The node's address as the installs and calls reach it (through the relay).
    pub fn address(&self) -> Address {
        Address::from_str(&format!("orion+tcp://{}", self.relay.addr())).unwrap()
    }
}

pub async fn open_installs(dir: &Path, sessions: Arc<Sessions>) -> SlotInstalls {
    let jobs = dir.join("jobs");
    let _ = std::fs::create_dir_all(&jobs);
    SlotInstalls::open(
        jobs.join("jobs.json"),
        sessions,
        Events::new(256),
        JobTiming {
            step_timeout: DEADLINE,
            backoff: Backoff {
                first: Duration::from_millis(10),
                cap: Duration::from_millis(50),
            },
        },
        DeviceHolds::new(),
        BOARD,
        jobs,
    )
    .await
    .expect("the job list opens")
}

/// A bundle for slot `id` at `version`, for the simulated board. `board` overrides the board id
/// the bundle claims (the wrong-board test).
pub fn bundle_bytes(id: &str, version: &str, board: Option<&str>) -> Vec<u8> {
    let rootfs = format!("rootfs of {id} at {version}\n")
        .repeat(900)
        .into_bytes();
    let boot: [(&str, &[u8]); 2] = [
        ("cmdline.txt", b"console=ttyS0 root=/dev/dm-0\n"),
        ("kernel.img", b"kernel bytes"),
    ];
    let mut manifest = atlas_sim_node::bundle::manifest(id, version, &rootfs, &boot);
    manifest["id"] = serde_json::Value::String(id.into());
    if let Some(board) = board {
        manifest["board"] = serde_json::Value::String(board.into());
    }
    atlas_sim_node::bundle::build(&manifest, None, None, &rootfs, &boot)
}

/// Writes a bundle to a file in `dir` and returns its path.
pub fn write_bundle(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("the bundle is written");
    path
}
