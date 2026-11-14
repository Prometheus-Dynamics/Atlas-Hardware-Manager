//! The test rig: a simulated board (Orion with Lemnos, `atlas-sim-node`), a relay in front of it
//! that can make the board leave and come back (a restart, as the address sees it), and the bundle
//! and environment the commands run with.
#![allow(dead_code)]

use std::io::{self, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use atlas_next::Env;
use atlas_session::Pause;
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use orion::node::Node;
use tokio::time;

/// The board id and serial of the simulated board. Its bundles are built for the board id.
pub const BOARD: &str = "simulated";
pub const SERIAL: &str = "0123abcd";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A scratch directory for one test, kept short: the node's socket path must fit a Unix socket.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atl-next-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The simulated board: a node with Lemnos that publishes the slot contract.
pub fn kind() -> Kind {
    Kind::WithLemnos(Hello {
        family: BOARD.into(),
        serial: SERIAL.into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    })
}

/// Starts a simulated board in `dir` on any free port, or on `port` when given.
pub fn start_node(dir: &Path, port: Option<u16>) -> SimNode {
    SimNode::start_with(&dir.join("node"), &kind(), port).expect("the node starts")
}

/// The board's identity as it answers now (its key and fingerprint), read on a fresh connection.
pub async fn identity(node: &SimNode) -> Node {
    time::timeout(DEADLINE, async {
        loop {
            if let Ok(orion) = node.connect("probe")
                && let Ok(identity) = orion.node()
            {
                return identity;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the node answers a connection")
}

/// A TCP relay in front of the node. `away` closes new connections at once; `cut` drops the live
/// ones. Together they are a board that leaves and comes back, as the address sees it.
#[derive(Debug)]
pub struct Relay {
    addr: SocketAddr,
    live: Arc<Mutex<Vec<TcpStream>>>,
    refusing: Arc<AtomicBool>,
}

impl Relay {
    pub fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the relay binds");
        let addr = listener.local_addr().expect("the relay has an address");
        let live = Arc::new(Mutex::new(Vec::new()));
        let refusing = Arc::new(AtomicBool::new(false));
        let (kept, refused) = (Arc::clone(&live), Arc::clone(&refusing));
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                if refused.load(Ordering::SeqCst) {
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                }
                let Ok(server) = TcpStream::connect(upstream) else {
                    continue;
                };
                let (Ok(client_cut), Ok(server_cut)) = (client.try_clone(), server.try_clone())
                else {
                    continue;
                };
                lock(&kept).push(client_cut);
                lock(&kept).push(server_cut);
                let (Ok(mut to_server), Ok(mut to_client)) =
                    (server.try_clone(), client.try_clone())
                else {
                    continue;
                };
                let (mut from_client, mut from_server) = (client, server);
                thread::spawn(move || {
                    let _ = io::copy(&mut from_client, &mut to_server);
                    let _ = to_server.shutdown(Shutdown::Both);
                });
                thread::spawn(move || {
                    let _ = io::copy(&mut from_server, &mut to_client);
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

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The address text a command takes for this relay.
    pub fn address(&self) -> String {
        format!("orion+tcp://{}", self.addr)
    }

    /// While `away`, the board is not reachable: new connections are closed at once.
    pub fn away(&self, away: bool) {
        self.refusing.store(away, Ordering::SeqCst);
    }

    /// Drops every live connection now.
    pub fn cut(&self) {
        for stream in std::mem::take(&mut *lock(&self.live)) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

/// A wait between a connection's tries that only yields: the monitor retries as soon as it can.
#[derive(Debug)]
pub struct YieldPause;

impl Pause for YieldPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        thread::yield_now();
    }
}

/// The environment a command runs in: its state in `state`, short polls.
pub fn env(state: &Path) -> Env {
    Env {
        state_dir: Some(state.to_path_buf()),
        pause: Arc::new(YieldPause),
        wait: Duration::from_secs(30),
        recheck: Duration::from_millis(20),
        stop: None,
    }
}

/// Runs one command line, as the binary does, and returns the status and what it printed.
pub async fn run(env: &Env, args: &[&str]) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    let code = atlas_next::execute(&args, env, &mut out).await;
    (code, String::from_utf8(out).expect("the output is text"))
}

/// Waits until `check` holds, within the simulated node's deadline.
pub async fn until(what: &str, mut check: impl FnMut() -> bool) {
    time::timeout(DEADLINE, async {
        while !check() {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// A slot bundle for `id` at `version`, for the simulated board. `board` overrides the board id the
/// bundle claims (the wrong-board case).
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

/// Writes a bundle file under `dir` and returns its path.
pub fn write_bundle(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).expect("the bundle directory is made");
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("the bundle file is created");
    file.write_all(bytes).expect("the bundle is written");
    path
}
