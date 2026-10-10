//! The NetworkTables page: viewers and the local server (nt.rs).

use std::net::IpAddr;

use orion_nt4::{Properties, Value};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use super::CmdResult;
use crate::nt::{NtFrame, NtServerFrame, NtServerInfo};
use crate::state::AppState;

/// Connects a viewer to `target`: a team number (its robot, 10.TE.AM.2) or
/// a host. Returns its id for [`nt_disconnect`].
#[tauri::command]
pub fn nt_connect(
    state: State<'_, AppState>,
    target: String,
    port: Option<u16>,
    on_frame: Channel<NtFrame>,
) -> u64 {
    state.nt.connect(&target, port, on_frame)
}

#[tauri::command]
pub fn nt_disconnect(state: State<'_, AppState>, id: u64) {
    state.nt.disconnect(id);
}

/// Starts the local NT4 server (port 5810 by default), restoring the topics
/// kept from last time (nt-server.json in the data directory); already
/// running: its state.
#[tauri::command]
pub async fn nt_server_start(
    state: State<'_, AppState>,
    port: Option<u16>,
) -> CmdResult<NtServerInfo> {
    // Topics marked "keep" come back the next time it starts.
    let persist = state.paths.data_dir.join("nt-server.json");
    state.nt.start_server(port, Some(persist)).await
}

#[tauri::command]
pub async fn nt_server_stop(state: State<'_, AppState>) -> CmdResult<()> {
    state.nt.stop_server().await;
    Ok(())
}

/// The local server's topics, clients and addresses, or null when it isn't
/// running.
#[tauri::command]
pub async fn nt_server_info(state: State<'_, AppState>) -> CmdResult<Option<NtServerInfo>> {
    Ok(state.nt.server_info().await)
}

/// Sends the local server's changes (topics, clients, values clients wrote)
/// to `on_frame` until it stops.
#[tauri::command]
pub async fn nt_server_watch(
    state: State<'_, AppState>,
    on_frame: Channel<NtServerFrame>,
) -> CmdResult<()> {
    state.nt.watch_server(on_frame).await
}

/// Creates a topic of `type_name`, or sets its value (`{type, value}`):
/// a new topic takes the value's type.
#[tauri::command]
pub async fn nt_server_set(
    state: State<'_, AppState>,
    name: String,
    type_name: Option<String>,
    value: Option<Value>,
) -> CmdResult<()> {
    state
        .nt
        .with_server(|server| {
            if let Some(type_name) = &type_name
                && server.topic(&name).is_none()
            {
                server.publish(&name, type_name, Properties::new())?;
            }
            if let Some(value) = value {
                server.set_value(&name, value)?;
            }
            Ok(())
        })
        .await
}

/// Keeps a topic across restarts of the server, or not.
#[tauri::command]
pub async fn nt_server_persistent(
    state: State<'_, AppState>,
    name: String,
    persistent: bool,
) -> CmdResult<()> {
    let mut update = Properties::new();
    update.insert("persistent".into(), persistent.into());
    update.insert("retained".into(), persistent.into());
    state
        .nt
        .with_server(|server| server.set_properties(&name, update))
        .await
}

#[tauri::command]
pub async fn nt_server_delete(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.nt.with_server(|server| server.delete(&name)).await
}

/// A camera Atlas knows, and this computer's address it reaches: what to
/// put in its NetworkTables server setting.
#[derive(Serialize)]
pub struct NtCameraAddress {
    pub name: String,
    pub device_ip: IpAddr,
    pub address: IpAddr,
}

/// For each online device Atlas reaches over IP, the address of this
/// computer on the way to it.
#[tauri::command]
pub fn nt_camera_addresses(state: State<'_, AppState>) -> Vec<NtCameraAddress> {
    let mut found: Vec<NtCameraAddress> = state
        .atlas
        .devices()
        .into_iter()
        .filter(|record| record.presence == atlas_core::Presence::Online)
        .filter_map(|record| {
            let host = reqwest_host(&record.identity.address)?;
            let device_ip: IpAddr = host.parse().ok()?;
            let address = atlas_image_server::local_address_for(device_ip)?;
            Some(NtCameraAddress {
                name: record.display_name(),
                device_ip,
                address,
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The host of a URL like `http://172.31.209.217:5899/...`, unbracketed.
fn reqwest_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split('/').next()?;
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().map(str::to_string);
    }
    Some(authority.split(':').next()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::reqwest_host;

    #[test]
    fn the_host_of_an_identity_url() {
        assert_eq!(
            reqwest_host("http://172.31.209.217:5899/.well-known/pd-device").as_deref(),
            Some("172.31.209.217")
        );
        assert_eq!(
            reqwest_host("http://[fe80::1]:5899/x").as_deref(),
            Some("fe80::1")
        );
        assert_eq!(reqwest_host("usb:001-4").as_deref(), Some("usb"));
    }
}
