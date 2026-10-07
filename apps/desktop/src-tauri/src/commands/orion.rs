//! Orion: where Atlas connects, how it enrolls, and how the connection is
//! doing. `None` when Orion is off for this session (simulated devices).

use tauri::State;

use super::CmdResult;
use crate::settings::AppPaths;
use crate::state::AppState;

pub use atlas_driver_orion::OrionConnection;
pub use atlas_image_server::ImageServerStatus;

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
    if let Some(orion) = &state.orion {
        orion.transport.set_url(url).await;
        orion.start_images_if_configured(&state.settings());
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

/// Where boards download update images from. `None` without Orion.
#[tauri::command]
pub fn image_server_status(state: State<'_, AppState>) -> Option<ImageServerStatus> {
    state.orion.as_ref().map(|orion| orion.images.status())
}

/// A host for image URLs: a name or address, without scheme, port, or path.
fn check_image_host(host: Option<String>) -> Result<Option<String>, String> {
    let Some(host) = host
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
    else {
        return Ok(None);
    };
    let bare_v6 = host.starts_with('[') && host.ends_with(']');
    let valid = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-._:[]".contains(c))
        && (bare_v6 || !host.contains(['[', ']']))
        && (bare_v6 || !host.contains(':'));
    if !valid {
        return Err(
            "The image host is a name or address like 192.168.1.20 or atlas-laptop.local, without http:// or a port"
                .into(),
        );
    }
    Ok(Some(host))
}

/// Saves the image server's port and host. A running server moves to the
/// new port at once; a port that can't be used shows in the status.
#[tauri::command]
pub async fn set_image_server(
    state: State<'_, AppState>,
    port: u16,
    host: Option<String>,
) -> CmdResult<Option<ImageServerStatus>> {
    if port == 0 {
        return Err("Choose a port from 1 to 65535".into());
    }
    let host = check_image_host(host)?;
    let mut settings = state.settings();
    settings.image_server_port = port;
    settings.image_host = host;
    settings.save(&AppPaths::resolve(false).settings_file)?;
    *state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings.clone();
    if let Some(orion) = &state.orion {
        // A failure is recorded in the status, which the panel shows.
        let _ = orion
            .images
            .reconfigure(crate::orion::image_config(&settings));
        orion.start_images_if_configured(&settings);
    }
    Ok(image_server_status(state))
}

#[cfg(test)]
mod tests {
    use super::check_image_host;

    #[test]
    fn image_hosts_are_bare_names_or_addresses() {
        let ok = |host: &str| check_image_host(Some(host.into())).unwrap();
        assert_eq!(ok("  "), None);
        assert_eq!(ok(" 192.168.1.20 ").as_deref(), Some("192.168.1.20"));
        assert_eq!(
            ok("atlas-laptop.local").as_deref(),
            Some("atlas-laptop.local")
        );
        assert_eq!(ok("[fd00::2]").as_deref(), Some("[fd00::2]"));
        for bad in ["http://x", "x:7700", "x/y", "a b", "fd00::2"] {
            assert!(check_image_host(Some(bad.into())).is_err(), "{bad}");
        }
    }
}
