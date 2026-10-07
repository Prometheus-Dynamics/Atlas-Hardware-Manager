//! `atlas.log` in the data directory: what Atlas did and what went wrong,
//! kept across restarts for debugging on real hardware. Job steps and
//! logs, scan warnings, fleet activity, and panics (with their location)
//! are written; live readings are not. Rotates to `atlas.log.1` at 5 MB.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use atlas_core::Event;

const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct LogFile {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

static LOG: OnceLock<LogFile> = OnceLock::new();

/// Opens the log and routes panics into it. Call once at startup.
pub fn init(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = LOG.set(LogFile {
        path: path.to_path_buf(),
        file: Mutex::new(open(path)),
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|at| format!(" at {}:{}", at.file(), at.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("no message");
        line("PANIC", &format!("{message}{location}"));
        previous(info);
    }));
    line(
        "START",
        &format!(
            "Atlas {} on {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ),
    );
}

fn open(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn timestamp() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    format!("{ms}")
}

/// Appends one line. Never fails the caller: logging is best effort.
pub fn line(kind: &str, message: &str) {
    let Some(log) = LOG.get() else {
        return;
    };
    let mut guard = log
        .file
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if fs::metadata(&log.path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = fs::rename(&log.path, log.path.with_extension("log.1"));
        *guard = open(&log.path);
    }
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(
            file,
            "{} {kind:<8} {}",
            timestamp(),
            message.replace('\n', " | ")
        );
    }
}

/// Writes the events worth keeping.
pub fn event(event: &Event) {
    match event {
        Event::ScanWarning { message } => line("WARN", message),
        Event::JobStarted { job, devices } => line(
            "JOB",
            &format!(
                "#{job} started for {}",
                devices
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Event::JobStep { job, device, step } => {
            line("JOB", &format!("#{job} {device} step {}", step.label()));
        }
        Event::JobLog {
            job,
            device,
            message,
        } => line("JOB", &format!("#{job} {device}: {message}")),
        Event::JobDevice {
            job,
            device,
            status,
        } if status.is_finished() => {
            line(
                "JOB",
                &format!("#{job} {device} {}: {status:?}", status.label()),
            );
        }
        Event::JobFinished {
            job,
            state,
            summary,
        } => {
            line("JOB", &format!("#{job} finished {state:?} {summary:?}"));
        }
        Event::Activity { entry } => line("ACTIVITY", &entry.message),
        Event::SelfTest { record } => {
            let checks = record.report.iter().flat_map(|report| &report.checks);
            for check in checks.filter(|check| check.status != atlas_driver::CheckStatus::Ok) {
                line(
                    "SELFTEST",
                    &format!(
                        "{} {} {:?}: {}",
                        record.device, check.id, check.status, check.message
                    ),
                );
            }
        }
        _ => {}
    }
}

/// The last `lines` lines of the log, for support bundles.
pub fn tail(lines: usize) -> Vec<String> {
    let Some(log) = LOG.get() else {
        return Vec::new();
    };
    let Ok(file) = File::open(&log.path) else {
        return Vec::new();
    };
    let all: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    all[all.len().saturating_sub(lines)..].to_vec()
}
