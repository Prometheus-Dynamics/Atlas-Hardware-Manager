//! Shared by the access tests: a simulated node with its shell, a carrier that reaches the
//! shell over the simulated frames, and waits that end at a generous deadline.

#![allow(dead_code)]

use std::fs::File;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use atlas_access::{Carrier, Channel, Control, Output, OutputEvent, PublicKey, ShellRequest};
use atlas_session::{Handle, Pause, Presence, Sessions};
use atlas_sim_node::access::{ACK, DATA, EXIT, HEAD, RESIZE, read_frame, write_frame};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use orion::Address;

/// A pause that does not wait: the failure rule's delays are tested by the session. It only
/// yields, so the monitor does not spin while the node is gone.
#[derive(Debug)]
pub struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

pub fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("access-{name}"))
}

/// A simulated node that says it is `mode`, on `port` if given.
pub fn start_node(dir: &Path, mode: Mode, port: Option<u16>) -> SimNode {
    let kind = Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode,
        model: "test-model".into(),
        ..Hello::default()
    });
    SimNode::start_with(dir, &kind, port).expect("the simulated node starts")
}

pub fn fresh(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

pub fn sessions() -> Sessions {
    Sessions::new(Arc::new(NoPause))
}

pub fn handle_for(sessions: &Sessions, port: u16) -> Handle {
    let address = Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap();
    sessions.handle(&address).expect("the node is on")
}

/// Waits until the handle's monitor reports the node connected.
pub async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected")
    .expect("the monitor is running");
}

/// Waits until the handle's monitor reports the node lost.
pub async fn lost(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Lost(_))),
    )
    .await
    .expect("the node is reported lost")
    .expect("the monitor is running");
}

/// A fresh throwaway ed25519 public key, from the operating system's random bytes. Nothing is
/// committed: the bytes exist only while the test runs.
pub fn throwaway_key(comment: &str) -> PublicKey {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .expect("the operating system gives random bytes");
    PublicKey::ed25519(bytes, comment)
}

/// The carrier of the tests: it reaches the simulated shell server and counts its calls.
#[derive(Debug)]
pub struct SimCarrier {
    addr: SocketAddr,
    pub opens: AtomicUsize,
    pub installs: AtomicUsize,
}

impl SimCarrier {
    pub fn new(addr: SocketAddr) -> Arc<Self> {
        Arc::new(Self {
            addr,
            opens: AtomicUsize::new(0),
            installs: AtomicUsize::new(0),
        })
    }

    fn connect(&self) -> Result<TcpStream, String> {
        TcpStream::connect_timeout(&self.addr, Duration::from_secs(5)).map_err(|e| e.to_string())
    }
}

impl Carrier for SimCarrier {
    fn open_shell(&self, request: &ShellRequest) -> Result<Channel, String> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let mut stream = self.connect()?;
        let head = format!("shell {} {}", request.cols, request.rows);
        write_frame(&mut stream, HEAD, head.as_bytes()).map_err(|e| e.to_string())?;
        let (tag, words) = read_frame(&mut stream).map_err(|e| e.to_string())?;
        if tag != ACK || words != b"ok" {
            return Err(String::from_utf8_lossy(&words).into_owned());
        }
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Channel {
            control: Arc::new(SimControl { stream: writer }),
            output: Box::new(SimOutput { stream }),
        })
    }

    fn install_key(&self, key: &PublicKey) -> Result<(), String> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        let mut stream = self.connect()?;
        let head = format!("key {}", key.line());
        write_frame(&mut stream, HEAD, head.as_bytes()).map_err(|e| e.to_string())?;
        let (tag, words) = read_frame(&mut stream).map_err(|e| e.to_string())?;
        if tag == ACK && words == b"ok" {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&words).into_owned())
        }
    }
}

#[derive(Debug)]
struct SimControl {
    stream: TcpStream,
}

impl Control for SimControl {
    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        write_frame(&mut &self.stream, DATA, bytes).map_err(|e| e.to_string())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let text = format!("{cols} {rows}");
        write_frame(&mut &self.stream, RESIZE, text.as_bytes()).map_err(|e| e.to_string())
    }

    fn hang_up(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[derive(Debug)]
struct SimOutput {
    stream: TcpStream,
}

impl Output for SimOutput {
    fn next(&mut self) -> OutputEvent {
        match read_frame(&mut self.stream) {
            Ok((DATA, bytes)) => OutputEvent::Data(bytes),
            Ok((EXIT, code)) if code.len() == 4 => {
                OutputEvent::Exit(i32::from_be_bytes([code[0], code[1], code[2], code[3]]))
            }
            Ok((tag, _)) => OutputEvent::Broke(format!("unexpected frame {tag}")),
            Err(error) => OutputEvent::Broke(error.to_string()),
        }
    }
}

/// Waits until the node's live hello says its `state`: the node's own facts are published a
/// little after it starts listening, so a connect can come first. Polls under the deadline.
pub async fn stated(handle: &Handle) {
    let deadline = std::time::Instant::now() + DEADLINE;
    loop {
        if let Ok(node) = handle.with_client(|orion| orion.node()).await
            && node.facts.iter().any(|fact| fact.name == "state")
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the node never said its state"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
