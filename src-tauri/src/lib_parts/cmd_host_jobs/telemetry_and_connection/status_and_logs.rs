use super::*;

pub(crate) struct TelemetryConnectionService;

impl TelemetryConnectionService {
    pub(crate) fn stop_device_telemetry_stream(
        request: DeviceTelemetryStopRequest,
    ) -> Result<String, TelemetryServiceError> {
        stop_device_telemetry_stream_impl(request)
    }

    pub(crate) async fn get_connection_status(
    ) -> Result<ConnectionStatusSnapshot, TelemetryServiceError> {
        tauri::async_runtime::spawn_blocking(get_connection_status_blocking)
            .await
            .map_err(|error| TelemetryServiceError::Worker(error.to_string()))?
            .map_err(TelemetryServiceError::Operation)
    }

    pub(crate) async fn fetch_device_log(
        request: DeviceLogRequest,
    ) -> Result<DeviceLogResult, TelemetryServiceError> {
        tauri::async_runtime::spawn_blocking(move || fetch_device_log_blocking(request))
            .await
            .map_err(|error| TelemetryServiceError::Worker(error.to_string()))?
            .map_err(TelemetryServiceError::Operation)
    }
}

#[tauri::command]
pub(crate) fn stop_device_telemetry_stream(
    request: DeviceTelemetryStopRequest,
) -> Result<String, String> {
    TelemetryConnectionService::stop_device_telemetry_stream(request)
        .map_err(|error| error.to_string())
}

fn stop_device_telemetry_stream_impl(
    request: DeviceTelemetryStopRequest,
) -> Result<String, TelemetryServiceError> {
    let active_stream_id = active_telemetry_stream_id();
    if active_stream_id.is_none() {
        return Ok("No telemetry stream is active.".to_string());
    }

    if let Some(requested_stream_id) = request
        .stream_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if active_stream_id.as_deref() != Some(requested_stream_id) {
            return Ok("Telemetry stream id did not match the active stream.".to_string());
        }
    }

    set_active_telemetry_stream_id(None);
    Ok("Telemetry stream stop requested.".to_string())
}

#[tauri::command]
pub(crate) async fn get_connection_status() -> Result<ConnectionStatusSnapshot, String> {
    TelemetryConnectionService::get_connection_status()
        .await
        .map_err(|error| error.to_string())
}

fn get_connection_status_blocking() -> Result<ConnectionStatusSnapshot, String> {
    let neighbors = discover_network_neighbors_fast().unwrap_or_default();
    let active_neighbors = neighbors
        .iter()
        .filter(|neighbor| is_active_connection_neighbor_state(neighbor.state.as_deref()))
        .cloned()
        .collect::<Vec<_>>();

    let roborio_neighbor = find_roborio_neighbor(&active_neighbors)
        .filter(is_live_roborio_neighbor_candidate)
        .or_else(|| {
            find_roborio_neighbor(&neighbors)
                .filter(is_live_roborio_neighbor_candidate)
        });
    if let Some(neighbor) = roborio_neighbor {
        let interface = neighbor
            .interface
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let connected_over_usb = is_roborio_usb_link(&neighbor.ip, Some(&interface));
        let route = if connected_over_usb {
            "rio-usb".to_string()
        } else {
            "rio-network".to_string()
        };
        let label = if connected_over_usb {
            "Connected to roboRIO over USB".to_string()
        } else {
            "Connected to roboRIO over Network".to_string()
        };
        let detail = format!("roboRIO candidate at {} on {}", neighbor.ip, interface);

        return Ok(ConnectionStatusSnapshot {
            connected: true,
            route,
            label,
            detail,
            target_ip: Some(neighbor.ip.clone()),
            interface_name: Some(interface),
            generated_at_epoch_ms: epoch_ms(),
        });
    }

    let helios_candidates = discover_helios_ip_candidates().unwrap_or_default();
    let helios_candidate = helios_candidates
        .iter()
        .find(|candidate| {
            is_active_connection_neighbor_state(candidate.state.as_deref())
                && is_host_port_responsive(&candidate.ip, &[5801, 5800, 80, 443])
        })
        .or_else(|| {
            helios_candidates
                .iter()
                .find(|candidate| is_host_port_responsive(&candidate.ip, &[5801, 5800, 80, 443]))
        });
    if let Some(candidate) = helios_candidate {
        let (route, label) = if candidate.is_usb_link {
            (
                "helios-usb".to_string(),
                "Connected to HeliOS over USB IP".to_string(),
            )
        } else {
            (
                "helios-network".to_string(),
                "Connected to HeliOS over Network IP".to_string(),
            )
        };

        return Ok(ConnectionStatusSnapshot {
            connected: true,
            route,
            label,
            detail: format!(
                "HeliOS candidate at {} on {}",
                candidate.ip, candidate.interface
            ),
            target_ip: Some(candidate.ip.clone()),
            interface_name: Some(candidate.interface.clone()),
            generated_at_epoch_ms: epoch_ms(),
        });
    }

    let usb_peers = active_neighbors
        .iter()
        .filter_map(|neighbor| {
            let interface = neighbor.interface.clone()?;
            if should_ignore_interface(&interface) {
                return None;
            }
            if !has_explicit_usb_interface_name(&interface) {
                return None;
            }
            Some((neighbor, interface))
        })
        .collect::<Vec<_>>();

    let detail = if let Some((neighbor, interface)) = usb_peers.first() {
        format!(
            "USB network peer {} on {} is visible, but no roboRIO/HeliOS route was verified.",
            neighbor.ip, interface
        )
    } else {
        "No active roboRIO or HeliOS network path found.".to_string()
    };

    Ok(ConnectionStatusSnapshot {
        connected: false,
        route: "disconnected".to_string(),
        label: "No robot connection detected".to_string(),
        detail,
        target_ip: None,
        interface_name: None,
        generated_at_epoch_ms: epoch_ms(),
    })
}

pub(crate) fn is_active_connection_neighbor_state(state: Option<&str>) -> bool {
    matches!(
        state.map(|value| value.to_ascii_uppercase()),
        Some(state) if matches!(state.as_str(), "REACHABLE" | "DELAY" | "PROBE" | "PERMANENT" | "STALE")
    )
}

#[tauri::command]
pub(crate) async fn fetch_device_log(request: DeviceLogRequest) -> Result<DeviceLogResult, String> {
    TelemetryConnectionService::fetch_device_log(request)
        .await
        .map_err(|error| error.to_string())
}
