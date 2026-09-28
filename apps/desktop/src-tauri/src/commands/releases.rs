use std::path::PathBuf;

use atlas_driver::Family;
use atlas_release::{ReleaseEntry, RemoteSource};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::{CmdResult, text};
use crate::state::AppState;

/// Payload of the `atlas://download` event.
#[derive(Clone, Debug, Serialize)]
pub struct DownloadEvent {
    pub id: String,
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[tauri::command]
pub fn list_releases(state: State<'_, AppState>) -> Vec<ReleaseEntry> {
    state.releases.entries()
}

/// Adds an image or firmware file from disk. It is hashed now and
/// re-checked before every install.
#[tauri::command]
pub async fn add_local_release(
    state: State<'_, AppState>,
    path: PathBuf,
    family: Family,
    version: String,
) -> CmdResult<ReleaseEntry> {
    let version = version.trim().to_string();
    if version.is_empty() {
        return Err("Give the release a version, for example 2026.3.1 or dev.".into());
    }
    state
        .releases
        .add_local_file(&path, family, version)
        .await
        .map_err(text)
}

#[tauri::command]
pub fn remove_release(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.releases.remove(&id).map_err(text)
}

/// Fetches every release source. Returns warnings, for example manifests
/// rejected for a bad signature.
#[tauri::command]
pub async fn refresh_releases(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(state.releases.refresh().await)
}

#[tauri::command]
pub async fn download_release(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<ReleaseEntry> {
    let event_id = id.clone();
    state
        .releases
        .download(&id, move |progress| {
            let _ = app.emit(
                "atlas://download",
                DownloadEvent {
                    id: event_id.clone(),
                    downloaded: progress.downloaded,
                    total: progress.total,
                },
            );
        })
        .await
        .map_err(text)
}

#[tauri::command]
pub fn list_release_sources(state: State<'_, AppState>) -> Vec<RemoteSource> {
    state.releases.sources()
}

#[tauri::command]
pub fn set_release_source(state: State<'_, AppState>, source: RemoteSource) -> CmdResult<()> {
    state.releases.set_source(source).map_err(text)
}

#[tauri::command]
pub fn remove_release_source(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.releases.remove_source(&name).map_err(text)
}
