use atlas_driver::HealthCheck;
use serde::Serialize;
use tauri::{AppHandle, State};

use super::CmdResult;
use crate::settings::{AppPaths, AppSettings, SimScenario};
use crate::state::AppState;

#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub arch: String,
    /// The simulated scenario this session runs, if any.
    pub simulated: Option<SimScenario>,
    pub paths: AppPaths,
    pub startup_warnings: Vec<String>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        simulated: state.simulated,
        paths: state.paths.clone(),
        startup_warnings: state.startup_warnings.clone(),
    }
}

#[tauri::command]
pub async fn health_checks(state: State<'_, AppState>) -> CmdResult<Vec<HealthCheck>> {
    Ok(state.atlas.health_checks().await)
}

/// Runs a health check's fix action, such as installing USB access.
#[tauri::command]
pub async fn fix_health(state: State<'_, AppState>, action: String) -> CmdResult<String> {
    state
        .atlas
        .fix_health(&action)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings()
}

/// Saves settings. Returns true when a change applies only after a restart.
#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> CmdResult<bool> {
    let mut settings = settings;
    settings.scan_interval_ms = settings.scan_interval_ms.clamp(1000, 60_000);
    let needs_restart = settings.simulated != state.simulated;
    settings.save(&AppPaths::resolve(false).settings_file)?;
    *state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
    Ok(needs_restart)
}

#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
