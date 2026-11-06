//! One viewer's stream: a `hello`, the events it missed (after its seq), the
//! update state, then every new event and update-state change as it is
//! written, and a keepalive while nothing happens.
//!
//! Server-Sent Events, one message each:
//! - `event: hello`, `data: {"boot_id":..,"seq":<newest>|null,"time":<unix s>}`
//! - `id: <seq>`, `event: event`, `data: <the events.jsonl line>`
//! - `event: update`, `data: <update.json>`
//! - `: keepalive`
//!
//! The files are looked at every poll (their size and mtime), so nothing
//! depends on inotify; a viewer that reconnects sends `Last-Event-ID` and
//! gets what it missed.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::request::StreamRequest;

/// Where the board's files are (the package's BOARD_RUN_DIR and
/// BOARD_DATA_DIR).
#[derive(Clone, Debug)]
pub struct Paths {
    pub run_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let dir = |name: &str, default: &str| {
            std::env::var_os(name).map_or_else(|| PathBuf::from(default), PathBuf::from)
        };
        Self {
            run_dir: dir("BOARD_RUN_DIR", "/run/board"),
            data_dir: dir("BOARD_DATA_DIR", "/data/board"),
        }
    }

    /// Both event logs: /run's (when /data wasn't writable) and /data's.
    fn event_files(&self) -> [PathBuf; 2] {
        [
            self.run_dir.join("events.jsonl"),
            self.data_dir.join("events.jsonl"),
        ]
    }

    fn update_json(&self) -> PathBuf {
        self.run_dir.join("update.json")
    }
}

/// A file's size and mtime, to tell it changed without reading it.
fn mark(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The events with a seq, from both logs: (seq, line), in seq order. Lines
/// that aren't whole events (a write cut off) or have no seq are left out.
pub fn numbered_events(paths: &Paths) -> Vec<(u64, String)> {
    let mut events: Vec<(u64, String)> = paths
        .event_files()
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| {
                    let value: Value = serde_json::from_str(line).ok()?;
                    Some((value.get("seq")?.as_u64()?, line.to_string()))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    events.sort_by_key(|(seq, _)| *seq);
    events
}

/// update.json's state line, if there is one.
fn update_state(paths: &Paths) -> Option<String> {
    let text = fs::read_to_string(paths.update_json()).ok()?;
    let line = text.lines().next()?.trim();
    serde_json::from_str::<Value>(line).ok()?;
    Some(line.to_string())
}

/// This boot's id (the package's BOARD_BOOT_ID for tests), else empty.
pub fn boot_id() -> String {
    std::env::var("BOARD_BOOT_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| fs::read_to_string("/proc/sys/kernel/random/boot_id").ok())
        .map(|id| id.trim().to_string())
        .unwrap_or_default()
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// One viewer's stream.
pub struct Stream<W: Write> {
    out: W,
    paths: Paths,
    request: StreamRequest,
    keepalive: Duration,
    last_seq: u64,
    last_update: Option<String>,
    marks: Vec<Option<(u64, SystemTime)>>,
    last_write: Instant,
}

impl<W: Write> Stream<W> {
    pub fn new(out: W, paths: Paths, request: StreamRequest, keepalive: Duration) -> Self {
        Self {
            out,
            paths,
            request,
            keepalive,
            last_seq: 0,
            last_update: None,
            marks: Vec::new(),
            last_write: Instant::now(),
        }
    }

    fn send(&mut self, message: &str) -> io::Result<()> {
        self.out.write_all(message.as_bytes())?;
        self.out.flush()?;
        self.last_write = Instant::now();
        Ok(())
    }

    fn event_marks(&self) -> Vec<Option<(u64, SystemTime)>> {
        self.paths
            .event_files()
            .iter()
            .map(|path| mark(path))
            .collect()
    }

    /// The response head, `hello`, the missed events and the update state.
    pub fn start(&mut self, boot_id: &str) -> io::Result<()> {
        self.send(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )?;
        let events = numbered_events(&self.paths);
        let newest = events.last().map(|(seq, _)| *seq);
        let hello = serde_json::json!({ "boot_id": boot_id, "seq": newest, "time": now_s() });
        self.send(&format!("event: hello\ndata: {hello}\n\n"))?;
        // Without a seq to resume from, only what is new from here on.
        self.last_seq = self.request.after_seq.unwrap_or(newest.unwrap_or(0));
        self.marks = self.event_marks();
        if self.request.events {
            self.send_events_after(events)?;
        }
        if self.request.update {
            self.last_update = update_state(&self.paths);
            if let Some(state) = self.last_update.clone() {
                self.send(&format!("event: update\ndata: {state}\n\n"))?;
            }
        }
        Ok(())
    }

    fn send_events_after(&mut self, events: Vec<(u64, String)>) -> io::Result<()> {
        for (seq, line) in events {
            if seq > self.last_seq {
                self.send(&format!("id: {seq}\nevent: event\ndata: {line}\n\n"))?;
                self.last_seq = seq;
            }
        }
        Ok(())
    }

    /// Sends what changed since the last call, or a keepalive when it's due.
    /// An error means the viewer is gone.
    pub fn tick(&mut self) -> io::Result<()> {
        if self.request.events {
            let marks = self.event_marks();
            if marks != self.marks {
                self.marks = marks;
                let events = numbered_events(&self.paths);
                // A log that started over (its /data replaced) numbers from 1.
                if let Some((newest, _)) = events.last()
                    && *newest < self.last_seq
                {
                    self.last_seq = 0;
                }
                self.send_events_after(events)?;
            }
        }
        if self.request.update {
            let state = update_state(&self.paths);
            if state.is_some() && state != self.last_update {
                self.last_update = state.clone();
                if let Some(state) = state {
                    self.send(&format!("event: update\ndata: {state}\n\n"))?;
                }
            }
        }
        if self.last_write.elapsed() >= self.keepalive {
            self.send(": keepalive\n\n")?;
        }
        Ok(())
    }
}
