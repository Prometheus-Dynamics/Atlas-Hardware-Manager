//! The test rig: a relay in front of a simulated node's port that can throttle, cut and refuse
//! connections, and a source that generates its bytes, so no test holds a large file.
#![allow(dead_code)]

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// The longest any wait in these tests runs. Nothing is expected to take this long: it only
/// ends a test whose condition never comes true.
pub const DEADLINE: Duration = Duration::from_secs(120);
/// How often a condition is looked at.
pub const POLL: Duration = Duration::from_millis(5);

/// A throttle, a budget, a cut and a refusal, applied to the connections a relay makes.
#[derive(Debug)]
struct Control {
    /// Bytes per second each way; 0 is unthrottled.
    bytes_per_sec: AtomicU64,
    /// Client-to-node bytes the relay still passes, over all its connections; `u64::MAX` is no
    /// limit. A connection that reaches it waits, so a transfer stops at a known point however
    /// fast or slow the host runs.
    budget: AtomicU64,
    refusing: AtomicBool,
    stop: AtomicBool,
    /// Both ends of each live connection, kept so a cut can shut them down.
    live: Mutex<Vec<(TcpStream, TcpStream)>>,
}

/// A TCP relay to a node's port. Connections through it are throttled to a rate; `pass_up_to`
/// stops what the client sends after a number of bytes; `cut` drops every live connection;
/// `refuse` drops new ones as they arrive.
#[derive(Debug)]
pub struct Relay {
    addr: SocketAddr,
    control: Arc<Control>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Relay {
    /// A relay on a free loopback port, in front of `upstream`.
    pub fn start(upstream: SocketAddr, bytes_per_sec: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the relay binds");
        let addr = listener.local_addr().expect("the relay has an address");
        let control = Arc::new(Control {
            bytes_per_sec: AtomicU64::new(bytes_per_sec),
            budget: AtomicU64::new(u64::MAX),
            refusing: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            live: Mutex::new(Vec::new()),
        });
        let accepting = Arc::clone(&control);
        thread::spawn(move || accept(&listener, upstream, &accepting));
        Self { addr, control }
    }

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Changes the rate for the bytes that move from now on.
    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.control
            .bytes_per_sec
            .store(bytes_per_sec, Ordering::SeqCst);
    }

    /// Passes at most `bytes` more from the client to the node, then holds what the client sends
    /// until the budget is raised. `u64::MAX` lifts the limit.
    pub fn pass_up_to(&self, bytes: u64) {
        self.control.budget.store(bytes, Ordering::SeqCst);
    }

    /// Refuses (or stops refusing) new connections.
    pub fn refuse(&self, refusing: bool) {
        self.control.refusing.store(refusing, Ordering::SeqCst);
    }

    /// Drops every live connection now. The client sees its session end.
    pub fn cut(&self) {
        for (client, server) in std::mem::take(&mut *lock(&self.control.live)) {
            let _ = client.shutdown(Shutdown::Both);
            let _ = server.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
        self.cut();
    }
}

fn accept(listener: &TcpListener, upstream: SocketAddr, control: &Arc<Control>) {
    let _ = listener.set_nonblocking(true);
    while !control.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => {
                if control.refusing.load(Ordering::SeqCst) {
                    drop(client);
                    continue;
                }
                let _ = client.set_nonblocking(false);
                let Ok(server) = TcpStream::connect(upstream) else {
                    continue;
                };
                let (Ok(client_up), Ok(server_up), Ok(client_down), Ok(server_down)) = (
                    client.try_clone(),
                    server.try_clone(),
                    client.try_clone(),
                    server.try_clone(),
                ) else {
                    continue;
                };
                lock(&control.live).push((client_down, server_down));
                let (up, down) = (Arc::clone(control), Arc::clone(control));
                thread::spawn(move || pump(client_up, server_up, &up, true));
                thread::spawn(move || pump(server, client, &down, false));
            }
            Err(_) => thread::sleep(POLL),
        }
    }
}

/// Takes up to `want` bytes from the budget, waiting while it is spent. None: the relay stopped.
fn take_budget(control: &Control, want: usize) -> Option<usize> {
    loop {
        if control.stop.load(Ordering::SeqCst) {
            return None;
        }
        let left = control.budget.load(Ordering::SeqCst);
        if left == u64::MAX {
            return Some(want);
        }
        if left > 0 {
            let take = left.min(want as u64);
            if control
                .budget
                .compare_exchange(left, left - take, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Some(take as usize);
            }
            continue;
        }
        thread::sleep(POLL);
    }
}

/// Copies one direction, sleeping after each piece so the rate holds. The client-to-node
/// direction (`up`) also keeps to the budget.
fn pump(mut from: TcpStream, mut to: TcpStream, control: &Control, up: bool) {
    let mut buffer = vec![0_u8; 32 * 1024];
    'copy: loop {
        let read = match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut at = 0;
        while at < read {
            let take = if up {
                match take_budget(control, read - at) {
                    Some(take) => take,
                    None => break 'copy,
                }
            } else {
                read - at
            };
            if to.write_all(&buffer[at..at + take]).is_err() {
                break 'copy;
            }
            at += take;
        }
        let rate = control.bytes_per_sec.load(Ordering::SeqCst);
        if rate > 0 {
            thread::sleep(Duration::from_secs_f64(read as f64 / rate as f64));
        }
    }
    let _ = to.shutdown(Shutdown::Both);
    let _ = from.shutdown(Shutdown::Both);
}

const fn splitmix(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `size` bytes generated from `seed`, read and sought like a file. It holds none of them: each
/// byte is computed when it is read.
#[derive(Debug)]
pub struct Generated {
    size: u64,
    position: u64,
    seed: u64,
}

impl Generated {
    pub const fn new(size: u64, seed: u64) -> Self {
        Self {
            size,
            position: 0,
            seed,
        }
    }

    pub const fn size(&self) -> u64 {
        self.size
    }
}

impl Read for Generated {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.size - self.position.min(self.size);
        let count = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
        let mut word = 0_u64;
        for (i, byte) in buf[..count].iter_mut().enumerate() {
            let offset = self.position + i as u64;
            if i == 0 || offset.is_multiple_of(8) {
                word = splitmix(self.seed ^ (offset / 8));
            }
            *byte = (word >> (8 * (offset % 8))) as u8;
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for Generated {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let position = match to {
            SeekFrom::Start(at) => i128::from(at),
            SeekFrom::End(by) => i128::from(self.size) + i128::from(by),
            SeekFrom::Current(by) => i128::from(self.position) + i128::from(by),
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        Ok(self.position)
    }
}

/// Whether `data` is exactly what `Generated::new(size, seed)` produces.
pub fn is_generated(data: &[u8], size: u64, seed: u64) -> bool {
    if data.len() as u64 != size {
        return false;
    }
    let mut expected = Generated::new(size, seed);
    let mut buffer = vec![0_u8; 1 << 20];
    let mut at = 0_usize;
    loop {
        let read = expected.read(&mut buffer).expect("generated bytes");
        if read == 0 {
            return true;
        }
        if data[at..at + read] != buffer[..read] {
            return false;
        }
        at += read;
    }
}
