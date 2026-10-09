use std::fmt::Write as _;
use std::path::PathBuf;

use atlas_core::{ActivityEntry, DeviceRecord, HistoryEntry, ScanReport, SelfTestRecord};
use atlas_driver::{DeviceAction, DeviceKey, DeviceStatus, LogLine, Metric};
use tauri::State;

use super::{CmdResult, text};
use crate::state::AppState;

#[tauri::command]
pub async fn scan(state: State<'_, AppState>) -> CmdResult<ScanReport> {
    Ok(state.atlas.scan().await)
}

#[tauri::command]
pub fn list_devices(state: State<'_, AppState>) -> Vec<DeviceRecord> {
    state.atlas.devices()
}

#[tauri::command]
pub fn set_device_label(
    state: State<'_, AppState>,
    key: DeviceKey,
    label: Option<String>,
) -> CmdResult<()> {
    state.atlas.set_label(&key, label).map_err(text)
}

#[tauri::command]
pub fn set_device_robot(
    state: State<'_, AppState>,
    key: DeviceKey,
    robot: Option<String>,
) -> CmdResult<()> {
    state.atlas.set_device_robot(&key, robot).map_err(text)
}

#[tauri::command]
pub fn forget_device(state: State<'_, AppState>, key: DeviceKey) -> CmdResult<()> {
    state.atlas.forget_device(&key).map_err(text)
}

#[tauri::command]
pub fn device_actions(state: State<'_, AppState>, key: DeviceKey) -> CmdResult<Vec<DeviceAction>> {
    state.atlas.actions(&key).map_err(text)
}

#[tauri::command]
pub async fn run_device_action(
    state: State<'_, AppState>,
    key: DeviceKey,
    action: String,
) -> CmdResult<()> {
    state.atlas.run_action(&key, &action).await.map_err(text)
}

/// Runs the device's self-test and returns the kept result, also when a
/// check failed or the run couldn't finish.
#[tauri::command]
pub async fn run_selftest(state: State<'_, AppState>, key: DeviceKey) -> CmdResult<SelfTestRecord> {
    state.atlas.run_selftest(&key).await.map_err(text)
}

/// The last self-test of this device's board, if it ever ran.
#[tauri::command]
pub fn device_selftest(state: State<'_, AppState>, key: DeviceKey) -> Option<SelfTestRecord> {
    state.atlas.selftest(&key)
}

/// Live readings, for devices that report telemetry.
#[tauri::command]
pub async fn device_telemetry(
    state: State<'_, AppState>,
    key: DeviceKey,
) -> CmdResult<Vec<Metric>> {
    state.atlas.telemetry(&key).await.map_err(text)
}

/// The last `lines` log lines, oldest first, for devices that report logs.
#[tauri::command]
pub async fn device_logs(
    state: State<'_, AppState>,
    key: DeviceKey,
    lines: usize,
) -> CmdResult<Vec<LogLine>> {
    state.atlas.logs(&key, lines).await.map_err(text)
}

/// What the device is doing now and how it is, for devices that report
/// `status`. Also brings its history up to date.
#[tauri::command]
pub async fn device_status(state: State<'_, AppState>, key: DeviceKey) -> CmdResult<DeviceStatus> {
    state.atlas.device_status(&key).await.map_err(text)
}

/// The device's history, newest first: what Atlas did and what its board's
/// event log says (Orion, someone on the board).
#[tauri::command]
pub fn device_history(
    state: State<'_, AppState>,
    key: DeviceKey,
    limit: usize,
) -> Vec<HistoryEntry> {
    state.atlas.device_history(&key, limit)
}

/// Fleet history, newest first.
#[tauri::command]
pub fn list_activity(state: State<'_, AppState>, limit: usize) -> Vec<ActivityEntry> {
    state.atlas.activity(limit.clamp(1, 1000))
}

/// Writes what Atlas knows about a device to a text file for a bug report:
/// identity, readings, recent logs, and its history. Parts the device does
/// not offer are left out.
#[tauri::command]
pub async fn save_support_bundle(
    state: State<'_, AppState>,
    key: DeviceKey,
    path: PathBuf,
) -> CmdResult<()> {
    let atlas = &state.atlas;
    let record = atlas
        .device(&key)
        .ok_or_else(|| format!("no device {key} in the inventory"))?;
    let mut out = String::new();
    let _ = writeln!(out, "Atlas support bundle for {}", record.display_name());
    let _ = writeln!(
        out,
        "Atlas {} on {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    let _ = writeln!(out, "\n== Device\n{}", pretty(&record));
    if let Ok(metrics) = atlas.telemetry(&key).await {
        let _ = writeln!(out, "\n== Readings");
        for metric in metrics {
            let unit = metric.unit.unwrap_or_default();
            let _ = writeln!(out, "{}: {} {unit}", metric.label, metric.value);
        }
    }
    if let Ok(lines) = atlas.logs(&key, 500).await {
        let _ = writeln!(out, "\n== Logs");
        for line in lines {
            let source = line.source.unwrap_or_default();
            let at = line.at_ms.map(|ms| ms.to_string()).unwrap_or_default();
            let _ = writeln!(out, "{at} {:?} [{source}] {}", line.level, line.message);
        }
    }
    if let Some(run) = atlas.selftest(&key) {
        let _ = writeln!(out, "\n== Last self-test\n{}", pretty(&run));
    }
    if let Ok(status) = atlas.device_status(&key).await {
        let _ = writeln!(out, "\n== Status\n{}", pretty(&status));
    }
    let recent = crate::logfile::tail(300);
    if !recent.is_empty() {
        let _ = writeln!(out, "\n== Atlas log (last {} lines)", recent.len());
        for line in recent {
            let _ = writeln!(out, "{line}");
        }
    }
    let _ = writeln!(out, "\n== History");
    for entry in atlas.device_history(&key, 2000) {
        let _ = writeln!(
            out,
            "{} {:?} {:?}/{:?} {} {}",
            entry.at_ms, entry.level, entry.origin, entry.source, entry.kind, entry.message
        );
    }
    std::fs::write(&path, out)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}
