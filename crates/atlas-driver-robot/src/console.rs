//! The robot program's console, as WPILib's RioLog reads it: TCP port 1741
//! on the roboRIO, open to anyone, streaming what the program prints and
//! the errors and warnings it reports, from the moment you connect (no
//! history).
//!
//! Each message is a frame: a big-endian `u16` length (of what follows),
//! a tag byte, and the message:
//!
//! - tag 12, a print: `f32` timestamp (seconds since the program started),
//!   `u16` sequence number, then the text;
//! - tag 11, an error or warning: timestamp, sequence number, `u16` times
//!   it occurred, `i32` code, a flags byte (bit 0: an error, else a
//!   warning), then three strings, each a `u16` length and its bytes:
//!   details, location, call stack.
//!
//! Anything else is skipped by its length.

use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atlas_driver::{DriverError, Identity, LogLevel, LogLine, LogsCapability};
use tokio::io::AsyncReadExt;

use crate::roborio::CONSOLE_PORT;

const TAG_ERROR: u8 = 11;
const TAG_PRINT: u8 = 12;
/// Lines a console tap keeps.
const KEEP: usize = 2000;
/// A tap nobody has read for this long disconnects.
const IDLE: Duration = Duration::from_secs(120);
/// How long a tap waits before reconnecting.
const RETRY: Duration = Duration::from_secs(5);

/// One console message.
#[derive(Clone, Debug, PartialEq)]
pub enum ConsoleMessage {
    Print {
        /// Seconds since the robot program started.
        timestamp: f32,
        seq: u16,
        text: String,
    },
    Report {
        timestamp: f32,
        seq: u16,
        error: bool,
        code: i32,
        occurrences: u16,
        details: String,
        location: String,
        call_stack: String,
    },
}

impl ConsoleMessage {
    /// As a log line: prints at info, reports as errors or warnings.
    pub fn to_log_line(&self) -> LogLine {
        match self {
            Self::Print { text, .. } => LogLine {
                at_ms: None,
                level: LogLevel::Info,
                source: Some("console".into()),
                message: text.trim_end().to_string(),
            },
            Self::Report {
                error,
                code,
                details,
                location,
                ..
            } => LogLine {
                at_ms: None,
                level: if *error {
                    LogLevel::Error
                } else {
                    LogLevel::Warning
                },
                source: Some(if location.is_empty() {
                    "console".into()
                } else {
                    location.clone()
                }),
                message: format!("{} ({code})", details.trim()),
            },
        }
    }

    /// One line of text, as a file of the console reads.
    pub fn to_text(&self) -> String {
        match self {
            Self::Print {
                timestamp, text, ..
            } => {
                format!("[{timestamp:>9.3}] {}", text.trim_end())
            }
            Self::Report {
                timestamp,
                error,
                code,
                details,
                location,
                call_stack,
                ..
            } => {
                let kind = if *error { "ERROR" } else { "WARNING" };
                let mut line = format!("[{timestamp:>9.3}] {kind} {code}: {}", details.trim_end());
                if !location.is_empty() {
                    line.push_str(&format!(" at {location}"));
                }
                if !call_stack.trim().is_empty() {
                    line.push('\n');
                    line.push_str(call_stack.trim_end());
                }
                line
            }
        }
    }
}

/// Reads whole frames from a byte stream; keeps a partial one for later.
#[derive(Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
}

impl FrameReader {
    /// Adds bytes and returns the messages they complete.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<ConsoleMessage> {
        self.buffer.extend_from_slice(bytes);
        let mut messages = Vec::new();
        let mut at = 0;
        while self.buffer.len() >= at + 2 {
            let len = usize::from(u16::from_be_bytes([self.buffer[at], self.buffer[at + 1]]));
            if self.buffer.len() < at + 2 + len {
                break;
            }
            let frame = &self.buffer[at + 2..at + 2 + len];
            at += 2 + len;
            if let Some(message) = parse_frame(frame) {
                messages.push(message);
            }
        }
        self.buffer.drain(..at);
        messages
    }
}

struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.take(4)
            .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Option<f32> {
        self.take(4)
            .map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn string(&mut self) -> Option<String> {
        let len = usize::from(self.u16()?);
        self.take(len)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }
    fn rest(&mut self) -> String {
        let text = String::from_utf8_lossy(self.0).into_owned();
        self.0 = &[];
        text
    }
}

fn parse_frame(frame: &[u8]) -> Option<ConsoleMessage> {
    let mut cursor = Cursor(frame);
    match cursor.u8()? {
        TAG_PRINT => Some(ConsoleMessage::Print {
            timestamp: cursor.f32()?,
            seq: cursor.u16()?,
            text: cursor.rest(),
        }),
        TAG_ERROR => {
            let timestamp = cursor.f32()?;
            let seq = cursor.u16()?;
            let occurrences = cursor.u16()?;
            let code = cursor.i32()?;
            let flags = cursor.u8()?;
            Some(ConsoleMessage::Report {
                timestamp,
                seq,
                error: flags & 1 != 0,
                code,
                occurrences,
                details: cursor.string()?,
                location: cursor.string().unwrap_or_default(),
                call_stack: cursor.string().unwrap_or_default(),
            })
        }
        _ => None,
    }
}

/// Reads the console at `ip` for `duration`, handing each message to
/// `each`. Returns how many arrived.
pub async fn capture(
    ip: Ipv4Addr,
    duration: Duration,
    mut each: impl FnMut(&ConsoleMessage),
) -> Result<usize, String> {
    let connect = tokio::net::TcpStream::connect((ip, CONSOLE_PORT));
    let mut stream = tokio::time::timeout(Duration::from_secs(3), connect)
        .await
        .map_err(|_| format!("{ip}:{CONSOLE_PORT} did not accept a connection"))?
        .map_err(|error| format!("{ip}:{CONSOLE_PORT}: {error}"))?;
    let deadline = tokio::time::Instant::now() + duration;
    let mut reader = FrameReader::default();
    let mut count = 0;
    let mut buf = [0u8; 4096];
    loop {
        let read = tokio::time::timeout_at(deadline, stream.read(&mut buf)).await;
        match read {
            Err(_) => return Ok(count),
            Ok(Ok(0)) => return Ok(count),
            Ok(Ok(n)) => {
                for message in reader.push(&buf[..n]) {
                    each(&message);
                    count += 1;
                }
            }
            Ok(Err(error)) => return Err(format!("{ip}:{CONSOLE_PORT}: {error}")),
        }
    }
}

/// A console kept open in the background while someone reads it, so the
/// device page's Logs tab has what the program printed since it opened.
struct Tap {
    lines: Mutex<VecDeque<LogLine>>,
    read_at: Mutex<Instant>,
}

impl Tap {
    fn touch(&self) {
        *self.read_at.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
    }

    fn idle(&self) -> bool {
        self.read_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed()
            >= IDLE
    }

    fn push(&self, line: LogLine) {
        let mut lines = self.lines.lock().unwrap_or_else(|p| p.into_inner());
        if lines.len() == KEEP {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    fn tail(&self, n: usize) -> Vec<LogLine> {
        let lines = self.lines.lock().unwrap_or_else(|p| p.into_inner());
        lines
            .iter()
            .skip(lines.len().saturating_sub(n))
            .cloned()
            .collect()
    }
}

fn taps() -> &'static Mutex<HashMap<Ipv4Addr, Arc<Tap>>> {
    static TAPS: OnceLock<Mutex<HashMap<Ipv4Addr, Arc<Tap>>>> = OnceLock::new();
    TAPS.get_or_init(Mutex::default)
}

/// The tap on `ip`'s console, started if it isn't running.
fn tap(ip: Ipv4Addr) -> Arc<Tap> {
    let mut open = taps().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(tap) = open.get(&ip) {
        tap.touch();
        return Arc::clone(tap);
    }
    let tap = Arc::new(Tap {
        lines: Mutex::new(VecDeque::new()),
        read_at: Mutex::new(Instant::now()),
    });
    open.insert(ip, Arc::clone(&tap));
    drop(open);
    let running = Arc::clone(&tap);
    tokio::spawn(async move {
        while !running.idle() {
            let sink = Arc::clone(&running);
            let _ = capture(ip, RETRY.max(IDLE / 4), |message| {
                sink.push(message.to_log_line());
            })
            .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        taps().lock().unwrap_or_else(|p| p.into_inner()).remove(&ip);
    });
    tap
}

/// The robot program's console as the device's logs: what it printed since
/// Atlas first asked (the stream has no history).
pub struct ConsoleLogs {
    pub ip: Ipv4Addr,
}

#[async_trait]
impl LogsCapability for ConsoleLogs {
    async fn tail(&self, _device: &Identity, lines: usize) -> Result<Vec<LogLine>, DriverError> {
        let tap = tap(self.ip);
        let mut out = tap.tail(lines);
        if out.is_empty() {
            out.push(LogLine {
                at_ms: None,
                level: LogLevel::Info,
                source: Some("atlas".into()),
                message: format!(
                    "Listening to the robot program's console ({}:{CONSOLE_PORT}); it sends only \
                     what is printed from now on.",
                    self.ip
                ),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print_frame(timestamp: f32, seq: u16, text: &str) -> Vec<u8> {
        let mut body = vec![TAG_PRINT];
        body.extend(timestamp.to_be_bytes());
        body.extend(seq.to_be_bytes());
        body.extend(text.as_bytes());
        framed(body)
    }

    fn framed(body: Vec<u8>) -> Vec<u8> {
        let mut frame = u16::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        frame.extend(body);
        frame
    }

    fn string(text: &str) -> Vec<u8> {
        let mut out = u16::try_from(text.len()).unwrap().to_be_bytes().to_vec();
        out.extend(text.as_bytes());
        out
    }

    #[test]
    fn prints_and_reports_parse_across_reads() {
        let mut bytes = print_frame(12.5, 1, "Robot program starting\n");
        let mut report = vec![TAG_ERROR];
        report.extend(13.0f32.to_be_bytes());
        report.extend(2u16.to_be_bytes());
        report.extend(1u16.to_be_bytes());
        report.extend((-1154i32).to_be_bytes());
        report.push(1);
        report.extend(string("CAN frame not received"));
        report.extend(string("frc::Talon"));
        report.extend(string(""));
        bytes.extend(framed(report));
        // A frame of an unknown tag is skipped.
        bytes.extend(framed(vec![99, 1, 2, 3]));

        let mut reader = FrameReader::default();
        // Split mid-frame: nothing until the rest arrives.
        let (first, second) = bytes.split_at(5);
        assert!(reader.push(first).is_empty());
        let messages = reader.push(second);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0],
            ConsoleMessage::Print {
                timestamp: 12.5,
                seq: 1,
                text: "Robot program starting\n".into()
            }
        );
        let line = messages[1].to_log_line();
        assert_eq!(line.level, LogLevel::Error);
        assert_eq!(line.source.as_deref(), Some("frc::Talon"));
        assert_eq!(line.message, "CAN frame not received (-1154)");
        assert_eq!(
            messages[1].to_text(),
            "[   13.000] ERROR -1154: CAN frame not received at frc::Talon"
        );
        assert_eq!(messages[0].to_text(), "[   12.500] Robot program starting");
        assert!(reader.buffer.is_empty());
    }

    /// Two frames from team 6390's roboRIO (a Java robot program, FRC
    /// 2026): a print and a warning.
    #[test]
    fn a_real_console_stream_parses() {
        let bytes = include_bytes!("../tests/fixtures/roborio-console.bin");
        let messages = FrameReader::default().push(bytes);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].to_log_line().message,
            "CommandScheduler loop overrun"
        );
        let warning = messages[1].to_log_line();
        assert_eq!(warning.level, LogLevel::Warning);
        assert_eq!(warning.message, "buttons.run(): 0.000185s (1)");
        assert_eq!(
            warning.source.as_deref(),
            Some("edu.wpi.first.wpilibj.Tracer.lambda$printEpochs$0(Tracer.java:62)")
        );
    }

    /// A capture reads frames from a console that streams them.
    #[tokio::test]
    async fn a_capture_reads_what_the_console_sends() {
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.3", CONSOLE_PORT)).await else {
            return; // Port taken or no 127.0.0.3: nothing to test against.
        };
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(&print_frame(1.0, 1, "hello")).await;
                let _ = socket.write_all(&print_frame(2.0, 2, "world")).await;
            }
        });
        let mut texts = Vec::new();
        let count = capture(
            Ipv4Addr::new(127, 0, 0, 3),
            Duration::from_millis(500),
            |message| texts.push(message.to_log_line().message),
        )
        .await
        .unwrap();
        assert_eq!((count, texts), (2, vec!["hello".into(), "world".into()]));
    }
}
