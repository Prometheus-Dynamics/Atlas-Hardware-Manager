//! The simulated way in for a person: an interactive shell and a public key install.
//!
//! // Orion at dbd5fc08 has `Orion::shell` and `Orion::enroll`, but no production carrier is built
//! // on them yet (see `atlas-access`, carrier.rs). Until one is, this module stands in for the
//! // carrier the `atlas-access` crate calls. Its frames are this module's own, not Orion's, and it
//! // is not a node: it is a socket that a [`SimNode`](crate::SimNode) test runs next to.
//!
//! The shell is a line echo with a prompt, enough to prove bytes in and bytes out, a resize that
//! arrives, and an exit status: `exit N` ends it with status `N`. A shell that is cut (see
//! [`ShellServer::cut`]) ends at once, as a lost link does.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

/// Client to server, the first frame: `shell COLS ROWS` or `key LINE`.
pub const HEAD: u8 = b'H';
/// Both ways: bytes of the shell's input or output.
pub const DATA: u8 = b'D';
/// Client to server: `COLS ROWS` as text.
pub const RESIZE: u8 = b'R';
/// Server to client, the answer to the first frame: `ok`, or `refused WORDS`.
pub const ACK: u8 = b'A';
/// Server to client: the shell ended with this status, four bytes, big endian.
pub const EXIT: u8 = b'X';
/// The most a frame may carry.
pub const MAX_FRAME: usize = 64 * 1024;

/// The prefix every simulated ed25519 key blob starts with: the length and name `ssh-ed25519`.
const ED25519_BLOB_PREFIX: &str = "AAAAC3NzaC1lZDI1NTE5";

/// Writes one frame: a tag, its length, then its payload.
///
/// # Errors
///
/// If the payload is too long for a frame, or the stream fails.
pub fn write_frame(out: &mut impl Write, tag: u8, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too long",
        ));
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too long"))?;
    out.write_all(&[tag])?;
    out.write_all(&len.to_be_bytes())?;
    out.write_all(payload)?;
    out.flush()
}

/// Reads one frame.
///
/// # Errors
///
/// If the stream ends or fails, or a frame is longer than [`MAX_FRAME`].
pub fn read_frame(input: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0_u8; 5];
    input.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
    }
    let mut payload = vec![0_u8; len];
    input.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

#[derive(Default)]
struct State {
    connections: usize,
    shells: usize,
    keys: Vec<String>,
    open: Vec<TcpStream>,
}

struct Shared(Mutex<State>);

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The simulated shell and key install, on its own port.
pub struct ShellServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ShellServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ShellServer({})", self.addr)
    }
}

impl ShellServer {
    /// Listens on a free port of this machine.
    ///
    /// # Errors
    ///
    /// If no port can be listened on.
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared(Mutex::new(State::default())));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    if let Ok(copy) = stream.try_clone() {
                        let mut state = shared.state();
                        state.connections += 1;
                        state.open.push(copy);
                    }
                    let shared = shared.clone();
                    thread::spawn(move || serve(stream, &shared));
                }
            })
        };
        Ok(Self {
            addr,
            shared,
            stop,
            thread: Some(thread),
        })
    }

    /// Where the shell and the key install are reached.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// How many connections have been made, of any kind.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.shared.state().connections
    }

    /// How many shells have been started.
    #[must_use]
    pub fn shells(&self) -> usize {
        self.shared.state().shells
    }

    /// The public key lines installed, in the order they were installed.
    #[must_use]
    pub fn installed_keys(&self) -> Vec<String> {
        self.shared.state().keys.clone()
    }

    /// Cuts every open connection, as a link that is lost does. Nothing reopens them.
    pub fn cut(&self) {
        let open = std::mem::take(&mut self.shared.state().open);
        for stream in open {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for ShellServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop so it sees the stop.
        let _ = TcpStream::connect(self.addr);
        self.cut();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Ok((tag, head)) = read_frame(&mut stream) else {
        return;
    };
    let text = String::from_utf8_lossy(&head).into_owned();
    if tag != HEAD {
        let _ = write_frame(&mut stream, ACK, b"refused not a request");
        return;
    }
    if let Some(line) = text.strip_prefix("key ") {
        if plausible_key(line) {
            shared.state().keys.push(line.to_owned());
            let _ = write_frame(&mut stream, ACK, b"ok");
        } else {
            let _ = write_frame(&mut stream, ACK, b"refused malformed key");
        }
        return;
    }
    if text.strip_prefix("shell ").is_some() {
        if write_frame(&mut stream, ACK, b"ok").is_err() {
            return;
        }
        shared.state().shells += 1;
        run_shell(stream);
        return;
    }
    let _ = write_frame(&mut stream, ACK, b"refused unknown request");
}

/// The node's own check of a key line: the form only, which the client has already checked.
fn plausible_key(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("ssh-ed25519")
        && words
            .next()
            .is_some_and(|blob| blob.len() == 68 && blob.starts_with(ED25519_BLOB_PREFIX))
}

/// The shell: echoes what it is given, answers each line, resizes, and ends on `exit N`.
fn run_shell(mut stream: TcpStream) {
    let mut line = Vec::new();
    loop {
        let Ok((tag, payload)) = read_frame(&mut stream) else {
            return;
        };
        match tag {
            DATA => {
                // A terminal echoes what it is given.
                if write_frame(&mut stream, DATA, &payload).is_err() {
                    return;
                }
                for byte in payload {
                    match byte {
                        b'\r' => {}
                        b'\n' => {
                            let text = String::from_utf8_lossy(&line).into_owned();
                            line.clear();
                            if let Some(code) = exit_status(&text) {
                                let _ = write_frame(&mut stream, EXIT, &code.to_be_bytes());
                                return;
                            }
                            let reply = format!("\r\n=> {text}\r\nsim$ ");
                            if write_frame(&mut stream, DATA, reply.as_bytes()).is_err() {
                                return;
                            }
                        }
                        other => line.push(other),
                    }
                }
            }
            RESIZE => {
                let text = String::from_utf8_lossy(&payload).into_owned();
                let reply = format!("size {}\r\n", text.trim().replace(' ', "x"));
                if write_frame(&mut stream, DATA, reply.as_bytes()).is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
}

/// `exit` and `exit N` end the shell with status N (0 if none is given).
fn exit_status(line: &str) -> Option<i32> {
    let rest = line.trim().strip_prefix("exit")?;
    if rest.is_empty() {
        return Some(0);
    }
    if !rest.starts_with(' ') {
        return None;
    }
    rest.trim().parse().ok()
}
