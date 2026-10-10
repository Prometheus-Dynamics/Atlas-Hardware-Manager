use std::fmt::Write as _;
use std::path::PathBuf;

use atlas_core::{ActivityEntry, DeviceRecord, HistoryEntry, ScanReport, SelfTestRecord};
use atlas_driver::{
    CalibrationStatus, DeviceAction, DeviceKey, DeviceStatus, FrameSink, HardwareCommand,
    HardwareFrame, LogLine, Metric,
};
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

/// Runs a command on one of the board's devices (set, restore, release); a
/// set answers the value the device applied.
#[tauri::command]
pub async fn control_hardware(
    state: State<'_, AppState>,
    key: DeviceKey,
    hardware: String,
    command: HardwareCommand,
) -> CmdResult<Option<f64>> {
    state
        .atlas
        .control_hardware(&key, &hardware, command)
        .await
        .map_err(text)
}

/// The calibration state of one of the board's devices (an IMU, a
/// magnetometer).
#[tauri::command]
pub async fn calibration_status(
    state: State<'_, AppState>,
    key: DeviceKey,
    hardware: String,
) -> CmdResult<CalibrationStatus> {
    state
        .atlas
        .calibration_status(&key, &hardware)
        .await
        .map_err(text)
}

/// Starts a live stream of the board devices `devices` (id, period ms) into
/// `on_frame`; returns its id for [`stop_hardware_stream`], or `None` when
/// the board has no live stream (its status snapshot is all there is). A
/// stream that ends sends a `gone` frame.
#[tauri::command]
pub async fn start_hardware_stream(
    state: State<'_, AppState>,
    key: DeviceKey,
    devices: Vec<(String, u32)>,
    on_frame: tauri::ipc::Channel<HardwareFrame>,
) -> CmdResult<Option<u64>> {
    if state.atlas.device(&key).is_none() {
        return Err(format!("no device {key}"));
    }
    let id = state
        .next_hardware_stream
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let atlas = state.atlas.clone();
    let channel = on_frame.clone();
    let sink: FrameSink = std::sync::Arc::new(move |frame| {
        let _ = channel.send(frame);
    });
    // The board answers at once when it has no stream: tell before
    // spawning, so the UI keeps its snapshot.
    let (tell, told) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let result = atlas.stream_hardware(&key, &devices, sink).await;
        let reason = match result {
            Ok(false) => {
                let _ = tell.send(false);
                return;
            }
            Ok(true) => "the board ended the stream".to_string(),
            Err(error) => error.to_string(),
        };
        let _ = tell.send(true);
        let _ = on_frame.send(HardwareFrame::Gone { reason });
    });
    // Streaming from the start, or no stream: whichever comes first.
    let streams = tokio::select! {
        told = told => told.unwrap_or(false),
        () = tokio::time::sleep(std::time::Duration::from_millis(300)) => true,
    };
    if !streams {
        return Ok(None);
    }
    state
        .hardware_streams
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id, task);
    Ok(Some(id))
}

/// Stops a live hardware stream (closing the board's connection, which
/// ends its lemnosd subscriptions).
#[tauri::command]
pub fn stop_hardware_stream(state: State<'_, AppState>, id: u64) {
    if let Some(task) = state
        .hardware_streams
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&id)
    {
        task.abort();
    }
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
