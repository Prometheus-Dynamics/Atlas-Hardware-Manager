//! The app's own log, and the support bundle.
//!
//! The bundle is one text file on this computer, made when the person asks: the app's version
//! and settings, what it knows of each device (no key material: the inventory holds none), its
//! jobs, and the app's log files. It is plain text so the person can read all of it before
//! sending it anywhere; Atlas sends it nowhere.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::call::{CallError, CallResult};
use crate::core::{Core, now_ms};

/// One part of a bundle, and how much of the file it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Part {
    pub name: String,
    pub bytes: usize,
}

/// A bundle that was made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Bundle {
    pub path: String,
    pub bytes: usize,
    pub parts: Vec<Part>,
}

/// What the Settings page says about the log and about sizes.
#[derive(Serialize)]
struct Info {
    /// The current log file, when the app has one.
    log_path: Option<String>,
    /// The size of every log file together.
    log_bytes: u64,
    /// The size of this program's own file.
    app_bytes: Option<u64>,
}

fn info(core: &Core) -> Info {
    Info {
        log_path: core.log().path().map(|path| path.display().to_string()),
        log_bytes: core
            .log()
            .files()
            .iter()
            .filter_map(|file| std::fs::metadata(file).ok())
            .map(|meta| meta.len())
            .sum(),
        app_bytes: std::env::current_exe()
            .ok()
            .and_then(|exe| std::fs::metadata(exe).ok())
            .map(|meta| meta.len()),
    }
}

/// The bundle's text and its parts.
#[must_use]
pub fn compose(core: &Core) -> (String, Vec<Part>) {
    let mut text = String::new();
    let mut parts = Vec::new();
    let mut part = |text: &mut String, name: &str, body: String| {
        let _ = writeln!(text, "===== {name} =====");
        text.push_str(body.trim_end());
        text.push_str("\n\n");
        parts.push(Part {
            name: name.to_owned(),
            bytes: body.len(),
        });
    };
    part(
        &mut text,
        "Atlas",
        format!(
            "version {}\ndevices {}\nmade at {} (ms since 1970)\n",
            crate::VERSION,
            core.mode().word(),
            now_ms()
        ),
    );
    let pretty = |value: &Value| serde_json::to_string_pretty(value).unwrap_or_default();
    let topics = core.topics();
    for (name, topic) in [
        ("Settings", "settings"),
        ("Devices and recent events", "inventory"),
        ("Scan", "scan"),
        ("Jobs", "jobs"),
    ] {
        part(
            &mut text,
            name,
            topics.get(topic).map(pretty).unwrap_or_default(),
        );
    }
    core.log().flush();
    for file in core.log().files() {
        let name = file
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let body = std::fs::read(&file)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_else(|error| format!("could not be read: {error}\n"));
        part(&mut text, &format!("App log: {name}"), body);
    }
    (text, parts)
}

fn make(core: &Core) -> Result<Bundle, String> {
    let dir: PathBuf = core
        .dirs()
        .map(|dirs| dirs.state.join("support"))
        .ok_or("Atlas has nowhere to write a file on this computer.")?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let (text, parts) = compose(core);
    let path = dir.join(format!("atlas-support-{}.txt", now_ms()));
    std::fs::write(&path, &text)
        .map_err(|error| format!("the bundle could not be written: {error}"))?;
    Ok(Bundle {
        path: path.display().to_string(),
        bytes: text.len(),
        parts,
    })
}

/// The `support.*` calls.
pub async fn call(core: &Core, name: &str, _arguments: Value) -> CallResult {
    match name {
        "support.info" => answer(&info(core)),
        "support.make_bundle" => {
            core.log().breadcrumb("support bundle asked for");
            let work = core.clone();
            let bundle = tokio::task::spawn_blocking(move || make(&work))
                .await
                .map_err(|error| CallError(error.to_string()))?
                .map_err(CallError)?;
            answer(&bundle)
        }
        "support.open_log_folder" => {
            let dir = core
                .log()
                .dir()
                .map(|dir| dir.display().to_string())
                .ok_or_else(|| CallError::from("This run of Atlas keeps no log."))?;
            core.opener().open(&dir).map_err(CallError)?;
            Ok(Value::Null)
        }
        // Opens the folder a bundle was written to.
        "support.open_bundle_folder" => {
            let dir = core
                .dirs()
                .map(|dirs| dirs.state.join("support").display().to_string())
                .ok_or_else(|| CallError::from("Atlas has nowhere it writes bundles."))?;
            core.opener().open(&dir).map_err(CallError)?;
            Ok(Value::Null)
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}

fn answer<T: Serialize>(value: &T) -> CallResult {
    serde_json::to_value(value).map_err(|error| CallError(error.to_string()))
}
