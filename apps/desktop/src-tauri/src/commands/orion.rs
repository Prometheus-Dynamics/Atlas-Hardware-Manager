//! Orion: where Atlas connects, how it enrolls, and how the connection is
//! doing. `None` when Orion is off for this session (simulated devices).

use tauri::State;

use super::CmdResult;
use crate::settings::AppPaths;
use crate::state::AppState;

pub use atlas_driver_orion::OrionConnection;

fn transport(state: &AppState) -> Option<&atlas_driver_orion::RemoteTransport> {
    state.orion.as_ref().map(|orion| orion.transport.as_ref())
}

#[tauri::command]
pub fn orion_connection(state: State<'_, AppState>) -> Option<OrionConnection> {
    transport(&state).map(atlas_driver_orion::RemoteTransport::connection)
}

/// Saves the Orion address, reconnects, and rescans so devices pick up
/// (or drop) Orion's capabilities.
#[tauri::command]
pub async fn set_orion_url(
    state: State<'_, AppState>,
    url: Option<String>,
) -> CmdResult<Option<OrionConnection>> {
    let url = url
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty());
    if let Some(url) = &url
        && !url.starts_with("orion+tcp://")
    {
        return Err("Orion addresses look like orion+tcp://host:port".into());
    }
    let mut settings = state.settings();
    settings.orion_url.clone_from(&url);
    settings.save(&AppPaths::resolve(false).settings_file)?;
    *state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
    if let Some(transport) = transport(&state) {
        transport.set_url(url).await;
    }
    state.atlas.scan().await;
    Ok(orion_connection(state))
}

/// Tries the connection now and reports how it went.
#[tauri::command]
pub async fn check_orion(state: State<'_, AppState>) -> CmdResult<Option<OrionConnection>> {
    if let Some(orion) = &state.orion {
        // Errors land in the connection state; the UI shows them there.
        let _ = orion.directory.refresh().await;
        state.atlas.scan().await;
    }
    Ok(orion_connection(state))
}

/// Shared-key enrollment, for nodes built with an enrollment key.
#[tauri::command]
pub async fn enroll_orion_with_key(
    state: State<'_, AppState>,
    key: String,
) -> CmdResult<Option<OrionConnection>> {
    if let Some(transport) = transport(&state) {
        transport
            .enroll_with_key(key.trim())
            .await
            .map_err(|error| error.to_string())?;
        state.atlas.scan().await;
    }
    Ok(orion_connection(state))
}
