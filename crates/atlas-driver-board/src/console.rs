//! The board's USB serial console as a root shell: every 1.0.7+ board has
//! one on its USB gadget (ACM, `serial-getty@ttyGS0` with root autologin),
//! so a board plugged into this computer is managed without SSH, a key, a
//! network or Orion. Holding the cable is the proof of access.
//!
//! One command at a time per port (a lock in this process and `flock` on
//! the tty for other Atlas processes). An exchange:
//!
//! 1. Ctrl-C (whatever ran in the console stops), then a probe line that
//!    prints a marker. No marker within [`PROBE`]: the console isn't there
//!    (getty masked, another program holding it) and nothing was run, so
//!    the caller may try another way.
//! 2. The script, base64 in short lines (the tty's line discipline limits
//!    and mangles long or odd lines), decoded on the board and run with
//!    `sh`: its stderr comes back line by line as it is written, its stdout
//!    as base64 at the end, then its exit status.
//!
//! Markers are printed with `printf '@X%s' <tag>`, so the echo of the
//! command line (when echo is on) never contains them. Terminal escapes (a
//! readline shell's bracketed paste, colour prompts) are stripped before
//! markers are looked for, anywhere in a line.
//!
//! A board has two consoles (ttyGS0 and ttyGS1). Atlas tries the last one
//! first, leaving the first to people, and the others when it doesn't
//! answer: one wedged console doesn't lock Atlas out. A login prompt (no
//! autologin) counts as not there: Atlas never logs in with a password.
//! On the board, a command runs under `timeout` when it has one.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use atlas_driver::DriverError;

/// How long the console has to answer the probe.
const PROBE: Duration = Duration::from_secs(3);
/// Base64 characters per line sent.
const CHUNK: usize = 384;

/// The board's consoles, in the order Atlas tries them.
#[derive(Clone, Debug)]
pub(crate) struct Console {
    paths: Vec<PathBuf>,
}

/// A line handler for stderr, as it comes.
pub(crate) type OnLine<'a> = &'a (dyn Fn(&str) + Send + Sync);
/// The same, handed to the console's thread.
pub(crate) type SharedOnLine = Arc<dyn Fn(&str) + Send + Sync>;

/// Why an exchange failed.
#[derive(Debug)]
pub(crate) enum ConsoleError {
    /// Nothing was run: the console didn't answer (or couldn't be opened);
    /// why, for logs and tests.
    NotThere(#[allow(dead_code)] String),
    /// The script ran (or may have): its error.
    Failed(DriverError),
}

impl Console {
    /// The console of the board whose USB serial is `serial`, when it is
    /// plugged into this computer.
    pub(crate) fn find(serial: &str) -> Option<Self> {
        let mut paths: Vec<PathBuf> = crate::serial::serial_consoles(serial)
            .into_iter()
            .map(PathBuf::from)
            .collect();
        paths.reverse();
        (!paths.is_empty()).then_some(Self { paths })
    }

    /// Runs `script` as root on the board; its stdout, as bytes.
    pub(crate) async fn run(
        &self,
        script: &str,
        timeout: Duration,
        on_line: Option<SharedOnLine>,
    ) -> Result<Vec<u8>, ConsoleError> {
        let paths = self.paths.clone();
        let script = script.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut why = Vec::new();
            for path in &paths {
                let lock = port_lock(path);
                let _held = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut port = match Tty::open(path, Instant::now() + PROBE) {
                    Ok(port) => port,
                    Err(error) => {
                        why.push(format!("{}: {error}", path.display()));
                        continue;
                    }
                };
                match exchange(&mut port, &script, &tag(), timeout, on_line.as_deref()) {
                    Err(ConsoleError::NotThere(reason)) => {
                        why.push(format!("{}: {reason}", path.display()));
                    }
                    other => return other,
                }
            }
            Err(ConsoleError::NotThere(why.join("; ")))
        })
        .await
        .map_err(|error| ConsoleError::Failed(DriverError::Other(error.to_string())))?
    }
}

/// One lock per tty in this process.
fn port_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());
    LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// A tag no earlier output holds.
fn tag() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    format!(
        "{:x}{:x}{:x}",
        std::process::id(),
        nanos,
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// What an exchange reads from and writes to.
pub(crate) trait Port {
    /// Bytes that came, waiting at most until `until` (empty: none yet).
    fn read_some(&mut self, until: Instant) -> std::io::Result<Vec<u8>>;
    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()>;
}

/// The lines to send for `script` (after the probe answered).
fn command_lines(script: &str, tag: &str, timeout: Duration) -> Vec<String> {
    // /run on the board (root's tmpfs); /tmp where /run isn't writable.
    let file = "$f";
    let mut lines = vec![format!(
        "f=/run/atlas-{tag}; [ -w /run ] || f=${{TMPDIR:-/tmp}}/atlas-{tag}; : > $f"
    )];
    let encoded = base64_encode(script.as_bytes());
    for chunk in encoded.as_bytes().chunks(CHUNK) {
        lines.push(format!(
            "echo {} >> {file}",
            std::str::from_utf8(chunk).unwrap_or_default()
        ));
    }
    let seconds = timeout.as_secs().max(1);
    lines.push(format!(
        "d={file}.d; mkdir -p $d; base64 -d {file} > $d/s; \
         t=; command -v timeout >/dev/null 2>&1 && t=\"timeout {seconds}\"; \
         {{ $t sh $d/s 2>&1 >$d/o </dev/null; echo $? > $d/rc; }} | \
         while IFS= read -r l; do printf '@E%s %s\\n' {tag} \"$l\"; done; \
         printf '@B%s\\n' {tag}; base64 $d/o; printf '@R%s %s\\n' {tag} \"$(cat $d/rc)\"; \
         rm -rf {file} $d; stty echo 2>/dev/null"
    ));
    lines
}

/// Runs `script` over `port`.
pub(crate) fn exchange(
    port: &mut dyn Port,
    script: &str,
    tag: &str,
    timeout: Duration,
    on_line: Option<OnLine<'_>>,
) -> Result<Vec<u8>, ConsoleError> {
    let mut reader = Lines::default();
    // Stop whatever runs, turn echo off, and ask for the probe's marker.
    let probe = format!("\x03\nstty -echo 2>/dev/null; printf '@P%s\\n' {tag}\n");
    port.send(probe.as_bytes())
        .map_err(|error| ConsoleError::NotThere(error.to_string()))?;
    let probe_marker = format!("@P{tag}");
    let until = Instant::now() + PROBE;
    loop {
        let bytes = port
            .read_some(until)
            .map_err(|error| ConsoleError::NotThere(error.to_string()))?;
        if reader
            .push(&bytes)
            .iter()
            .any(|line| line.contains(&probe_marker))
        {
            break;
        }
        if reader.at_login() {
            return Err(ConsoleError::NotThere(
                "it shows a login prompt (no autologin)".into(),
            ));
        }
        if Instant::now() >= until {
            return Err(ConsoleError::NotThere(
                "the board's console didn't answer".into(),
            ));
        }
    }

    for line in command_lines(script, tag, timeout) {
        port.send(format!("{line}\n").as_bytes())
            .map_err(|error| ConsoleError::Failed(gone(&error)))?;
    }
    let (err_marker, out_marker, rc_marker) =
        (format!("@E{tag} "), format!("@B{tag}"), format!("@R{tag} "));
    let mut stderr = String::new();
    let mut stdout_b64 = String::new();
    let mut in_stdout = false;
    let until = Instant::now() + timeout;
    loop {
        let bytes = port
            .read_some(until)
            .map_err(|error| ConsoleError::Failed(gone(&error)))?;
        for line in reader.push(&bytes) {
            if let Some(text) = line
                .find(&err_marker)
                .map(|at| &line[at + err_marker.len()..])
            {
                if let Some(on_line) = on_line {
                    on_line(text);
                }
                stderr.push_str(text);
                stderr.push('\n');
            } else if line.ends_with(&out_marker) {
                in_stdout = true;
            } else if let Some(code) = line
                .find(&rc_marker)
                .map(|at| &line[at + rc_marker.len()..])
            {
                let stdout = base64_decode(&stdout_b64).ok_or_else(|| {
                    ConsoleError::Failed(DriverError::Other(
                        "the board's console garbled the output".into(),
                    ))
                })?;
                return match code.trim().parse::<i32>() {
                    Ok(0) => Ok(stdout),
                    code => Err(ConsoleError::Failed(DriverError::Other({
                        let last = stderr.lines().last().unwrap_or("").trim();
                        if last.is_empty() {
                            format!(
                                "the board's command failed (exit {})",
                                code.map_or_else(|_| "?".into(), |c| c.to_string())
                            )
                        } else {
                            last.to_owned()
                        }
                    }))),
                };
            } else if in_stdout {
                stdout_b64.push_str(line.trim());
            }
        }
        if Instant::now() >= until {
            // Stop it, so the console is free for the next one. A board
            // that went quiet mid-command (it restarted, say) is
            // unreachable, not a command that failed: an update's `apply`
            // ends that way.
            let _ = port.send(b"\x03\n");
            return Err(ConsoleError::Failed(DriverError::Unreachable(
                "the board's console stopped answering".into(),
            )));
        }
    }
}

/// The tty went away (a restart): the board is no longer reachable.
fn gone(error: &std::io::Error) -> DriverError {
    DriverError::Unreachable(format!("the board's console went away: {error}"))
}

/// Splits what comes into lines, without `\r` or terminal escapes.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
    escape: Escape,
}

/// Where in a terminal escape the stream is.
#[derive(Default, Clone, Copy, PartialEq)]
enum Escape {
    #[default]
    None,
    /// After ESC.
    Start,
    /// In `ESC [ ...` up to its final byte.
    Csi,
    /// In `ESC ] ...` up to BEL or ESC \.
    Osc,
}

impl Lines {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            match (self.escape, byte) {
                (Escape::None, 0x1b) => self.escape = Escape::Start,
                (Escape::Start, b'[') => self.escape = Escape::Csi,
                (Escape::Start, b']') => self.escape = Escape::Osc,
                (Escape::Start, _) => self.escape = Escape::None,
                (Escape::Csi, 0x40..=0x7e) => self.escape = Escape::None,
                (Escape::Csi, _) => {}
                (Escape::Osc, 0x07) => self.escape = Escape::None,
                (Escape::Osc, 0x1b) => self.escape = Escape::Start,
                (Escape::Osc, _) => {}
                (Escape::None, b'\n') => {
                    lines.push(String::from_utf8_lossy(&self.partial).into_owned());
                    self.partial.clear();
                }
                (Escape::None, b'\r' | 0x07) => {}
                (Escape::None, other) => self.partial.push(other),
            }
        }
        lines
    }

    /// The line so far ends in a login prompt.
    fn at_login(&self) -> bool {
        String::from_utf8_lossy(&self.partial)
            .trim_end()
            .ends_with("login:")
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> shift) as usize & 63] as char
            } else {
                '='
            });
        }
    }
    out
}

pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut n = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\t' => continue,
            _ => return None,
        };
        n = (n << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
        }
    }
    Some(out)
}

/// The host side of the console: raw, reads time out every 100 ms, and
/// `flock`ed while open.
struct Tty {
    #[cfg(unix)]
    file: nix::fcntl::Flock<std::fs::File>,
    #[cfg(not(unix))]
    file: std::fs::File,
    /// The tty's path: when it is gone, the board left (a restart).
    path: PathBuf,
}

impl Tty {
    #[cfg(unix)]
    fn open(path: &Path, until: Instant) -> std::io::Result<Self> {
        use nix::fcntl::{Flock, FlockArg};
        use nix::sys::termios::{self, SetArg, SpecialCharacterIndices};
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(nix::libc::O_NOCTTY)
            .open(path)?;
        // Another Atlas process may be using it: wait for it, a while.
        let wait = until + Duration::from_secs(30);
        let file = loop {
            match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
                Ok(locked) => break locked,
                Err((back, errno)) => {
                    if Instant::now() >= wait {
                        return Err(std::io::Error::from(errno));
                    }
                    file = back;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        let mut attrs = termios::tcgetattr(&*file).map_err(std::io::Error::from)?;
        termios::cfmakeraw(&mut attrs);
        attrs.control_chars[SpecialCharacterIndices::VMIN as usize] = 0;
        attrs.control_chars[SpecialCharacterIndices::VTIME as usize] = 1;
        termios::tcsetattr(&*file, SetArg::TCSANOW, &attrs).map_err(std::io::Error::from)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    #[cfg(not(unix))]
    fn open(path: &Path, _until: Instant) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("{}: the console is Linux and macOS only", path.display()),
        ))
    }
}

impl Port for Tty {
    fn read_some(&mut self, until: Instant) -> std::io::Result<Vec<u8>> {
        let mut buf = [0u8; 4096];
        loop {
            // VTIME: a read returns after 100 ms with nothing.
            let n = (&*self.file).read(&mut buf)?;
            if n > 0 {
                return Ok(buf[..n].to_vec());
            }
            // A hung-up tty reads nothing at once, forever: the board's USB
            // gadget went away (it restarted).
            if !self.path.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "the board's console went away",
                ));
            }
            if Instant::now() >= until {
                return Ok(Vec::new());
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut file: &std::fs::File = &self.file;
        file.write_all(bytes)?;
        file.flush()
    }
}

#[cfg(test)]
mod tests;
