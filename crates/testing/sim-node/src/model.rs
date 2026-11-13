//! The state a simulated board keeps: its root slots, the data partition, the clock and
//! the readings. Pure data, so each rule can be checked without threads. The node applies
//! runs to it.

use std::collections::BTreeMap;

use crate::slots::{Slots, Status, code};

/// The readings at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Readings {
    pub temperature_c: f64,
    pub input_v: f64,
    pub fan_rpm: i64,
}

/// The state of one simulated board.
#[derive(Clone, Debug)]
pub struct Model {
    pub slots: Slots,
    /// The data partition: path to contents.
    pub data: BTreeMap<String, Vec<u8>>,
    /// How far the clock is off, in seconds. `None` once it is right.
    pub clock_offset_s: Option<i64>,
    /// Whether `set-clock` takes.
    pub clock_settable: bool,
    /// Advances on every tick of the node. Readings are a function of it.
    pub ticks: u64,
    pending_logs: Vec<String>,
    logs: Vec<String>,
}

const LOG_KEEP: usize = 500;

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    /// A board with one slot running, and a small data partition.
    #[must_use]
    pub fn new() -> Self {
        let mut model = Self {
            slots: Slots::new(),
            data: BTreeMap::from([("config/board.toml".into(), b"name = \"sim\"\n".to_vec())]),
            clock_offset_s: Some(-42),
            clock_settable: true,
            ticks: 0,
            pending_logs: Vec::new(),
            logs: Vec::new(),
        };
        model.log("booted slot-1");
        model
    }

    /// Adds a line to the log. It is also queued for the log topic.
    pub fn log(&mut self, line: &str) {
        self.pending_logs.push(line.to_string());
        self.logs.push(line.to_string());
        if self.logs.len() > LOG_KEEP {
            self.logs.remove(0);
        }
    }

    /// Moves the slot driver's events into the log, and returns the lines not yet taken.
    pub fn take_new_logs(&mut self) -> Vec<String> {
        for event in std::mem::take(&mut self.slots.events) {
            self.log(&event);
        }
        std::mem::take(&mut self.pending_logs)
    }

    /// Every log line kept, oldest first.
    #[must_use]
    pub fn logs(&self) -> &[String] {
        &self.logs
    }

    /// The readings now: a deterministic sweep, so a test knows what to expect.
    #[must_use]
    pub fn readings(&self) -> Readings {
        let step = (self.ticks % 40) as f64;
        Readings {
            temperature_c: 40.0 + step * 0.25,
            input_v: 12.0 + ((self.ticks % 10) as f64) * 0.01,
            fan_rpm: 1800 + ((self.ticks % 20) as i64) * 15,
        }
    }

    /// The boot and slot state in one line of text.
    #[must_use]
    pub fn status_text(&self) -> String {
        let slots = &self.slots;
        let trial = match &slots.trial {
            None => "none".to_string(),
            Some(trial) if trial.record.reached => format!("{} (booted, unconfirmed)", trial.id),
            Some(trial) => format!("{} (pending)", trial.id),
        };
        format!(
            "running={} default={} trial={} slots={}",
            slots.active.as_deref().unwrap_or("none"),
            slots.default.as_deref().unwrap_or("none"),
            trial,
            slots.entries.len(),
        )
    }

    /// The data partition as one blob, for a backup: each path, its length, then its bytes.
    #[must_use]
    pub fn data_backup(&self) -> Vec<u8> {
        let mut blob = Vec::new();
        for (path, bytes) in &self.data {
            blob.extend_from_slice(format!("{path}\n{}\n", bytes.len()).as_bytes());
            blob.extend_from_slice(bytes);
        }
        blob
    }

    /// Replaces the data partition with a backup made by [`Model::data_backup`].
    ///
    /// # Errors
    ///
    /// `INVALID_ARGUMENT` if the blob is not a backup. The partition is then left as it was.
    pub fn data_restore(&mut self, blob: &[u8]) -> Result<(), Status> {
        let bad = || {
            Status::new(
                code::INVALID_ARGUMENT,
                "that is not a backup of the data partition",
            )
        };
        let mut data = BTreeMap::new();
        let mut rest = blob;
        while !rest.is_empty() {
            let (path, after) = take_line(rest).ok_or_else(bad)?;
            let (length, after) = take_line(after).ok_or_else(bad)?;
            let length: usize = length.parse().map_err(|_| bad())?;
            if after.len() < length {
                return Err(bad());
            }
            data.insert(path.to_string(), after[..length].to_vec());
            rest = &after[length..];
        }
        self.data = data;
        self.log("data partition restored");
        Ok(())
    }
}

/// Splits off one line, without its newline. `None` if there is no newline.
fn take_line(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let end = bytes.iter().position(|byte| *byte == b'\n')?;
    let line = std::str::from_utf8(&bytes[..end]).ok()?;
    Some((line, &bytes[end + 1..]))
}
