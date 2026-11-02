//! `board-stream --serve-each <port> <program>`: a resident listener that
//! runs `program` once per TCP connection, the connection as its stdin and
//! stdout, as systemd's `Accept=yes` does but without a unit per connection
//! (board-http.service: the identity endpoint, 5899). Under QEMU a sandboxed
//! unit's start was 2.3–2.8 s of a 2.6–3.4 s identity request; the sandbox is
//! now set up once, for this process and every program it runs.
//!
//! What it keeps from the socket unit it replaces: `REMOTE_ADDR` and
//! `REMOTE_PORT` in the program's environment, at most [`MAX_CONNECTIONS`] at
//! once and [`MAX_PER_SOURCE`] from one address (others are closed unread),
//! and [`RUNTIME_MAX`] per program (then its process group is killed and the
//! connection shut, so a command it started can't keep the client waiting).

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_CONNECTIONS: usize = 16;
pub const MAX_PER_SOURCE: usize = 4;
pub const RUNTIME_MAX: Duration = Duration::from_secs(10);

/// Connections running now: in all, and per address.
#[derive(Default)]
struct Open {
    total: usize,
    by_source: HashMap<IpAddr, usize>,
}

/// One admitted connection; leaving lets the next one in.
struct Admitted {
    open: Arc<Mutex<Open>>,
    source: IpAddr,
}

impl Drop for Admitted {
    fn drop(&mut self) {
        let mut open = self.open.lock().unwrap_or_else(|p| p.into_inner());
        open.total = open.total.saturating_sub(1);
        if let Some(n) = open.by_source.get_mut(&self.source) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.by_source.remove(&self.source);
            }
        }
    }
}

fn admit(open: &Arc<Mutex<Open>>, source: IpAddr) -> Option<Admitted> {
    let mut state = open.lock().unwrap_or_else(|p| p.into_inner());
    let from_source = state.by_source.get(&source).copied().unwrap_or(0);
    if state.total >= MAX_CONNECTIONS || from_source >= MAX_PER_SOURCE {
        return None;
    }
    state.total += 1;
    *state.by_source.entry(source).or_default() += 1;
    Some(Admitted {
        open: open.clone(),
        source,
    })
}

/// An IPv4 client on the dual-stack socket as its IPv4 address, as systemd
/// reports it (identity-http strips `::ffff:` too).
fn plain(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

/// Runs `program` on one connection and waits for it, at most
/// [`RUNTIME_MAX`].
fn serve(stream: TcpStream, peer: SocketAddr, program: &PathBuf, runtime_max: Duration) {
    let _ = stream.set_nodelay(true);
    let (Ok(output), Ok(control)) = (stream.try_clone(), stream.try_clone()) else {
        return;
    };
    let spawned = Command::new(program)
        // Its own group, so a timeout reaches what it started too.
        .process_group(0)
        .env("REMOTE_ADDR", plain(peer.ip()).to_string())
        .env("REMOTE_PORT", peer.port().to_string())
        .stdin(Stdio::from(OwnedFd::from(stream)))
        .stdout(Stdio::from(OwnedFd::from(output)))
        .spawn();
    let Ok(mut child) = spawned else {
        return;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) if started.elapsed() >= runtime_max => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    let group = nix::unistd::Pid::from_raw(pid);
                    let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                let _ = control.shutdown(std::net::Shutdown::Both);
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Accepts on `listener` for good, one thread per admitted connection.
pub fn run(listener: TcpListener, program: PathBuf, runtime_max: Duration) -> io::Result<()> {
    let open = Arc::new(Mutex::new(Open::default()));
    let program = Arc::new(program);
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(accepted) => accepted,
            // A connection that went before it was taken; keep listening.
            Err(error) if error.kind() == io::ErrorKind::ConnectionAborted => continue,
            Err(error) => return Err(error),
        };
        // Over a limit: closed unread, as the socket unit's limits did.
        let Some(admitted) = admit(&open, plain(peer.ip())) else {
            continue;
        };
        let program = program.clone();
        let spawned = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                serve(stream, peer, &program, runtime_max);
                drop(admitted);
            });
        if spawned.is_err() {
            // No thread to run it on: the connection closes.
            continue;
        }
    }
}

/// The dual-stack listener on `port` (IPv4 clients too: bindv6only is 0),
/// or IPv4 alone on a kernel without IPv6.
pub fn listen(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((Ipv6Addr::UNSPECIFIED, port))
        .or_else(|_| TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)))
}

#[cfg(test)]
mod tests;
