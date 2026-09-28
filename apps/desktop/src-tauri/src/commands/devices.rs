use atlas_core::{DeviceRecord, ScanReport};
use atlas_driver::{DeviceAction, DeviceKey};
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
