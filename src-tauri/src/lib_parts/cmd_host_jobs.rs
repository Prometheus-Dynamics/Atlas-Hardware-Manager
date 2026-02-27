fn device_telemetry_stream_slot() -> &'static Mutex<Option<String>> {
    static ACTIVE_TELEMETRY_STREAM_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    ACTIVE_TELEMETRY_STREAM_ID.get_or_init(|| Mutex::new(None))
}

fn set_active_telemetry_stream_id(stream_id: Option<String>) {
    if let Ok(mut slot) = device_telemetry_stream_slot().lock() {
        *slot = stream_id;
    }
}

fn active_telemetry_stream_id() -> Option<String> {
    device_telemetry_stream_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}

fn telemetry_stream_is_active(stream_id: &str) -> bool {
    active_telemetry_stream_id().as_deref() == Some(stream_id)
}

#[allow(clippy::too_many_arguments)]
fn emit_device_telemetry_event(
    app: &tauri::AppHandle,
    stream_id: &str,
    status: &str,
    message: impl Into<String>,
    runtime_product: Option<String>,
    os_version: Option<String>,
    telemetry_summary: Option<String>,
    source_url: Option<String>,
) {
    let payload = DeviceTelemetryEvent {
        stream_id: stream_id.to_string(),
        status: status.to_string(),
        message: message.into(),
        timestamp_epoch_ms: epoch_ms(),
        runtime_product,
        os_version,
        telemetry_summary,
        source_url,
    };
    let _ = app.emit(DEVICE_TELEMETRY_EVENT, payload);
}

fn build_telemetry_stream_endpoints(ip: &str, runtime_product: Option<&str>) -> Vec<String> {
    let mut endpoints = Vec::new();
    let mut add_endpoint = |port: u16, path: &str| {
        endpoints.push(format!("ws://{ip}:{port}{path}"));
    };

    let product = runtime_product
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let is_photonvision = product.contains("photonvision");
    let is_helios_family = product.contains("helios") || product.contains("prometheus");
    let is_roborio = is_roborio_product(&product);

    if is_roborio {
        // roboRIO telemetry is typically polled over HTTP/SSH and not via websocket endpoints.
        return Vec::new();
    }

    if is_photonvision {
        // PhotonVision commonly publishes client telemetry here.
        add_endpoint(5800, "/websocket_data");
        add_endpoint(80, "/websocket_data");
    }

    if is_helios_family {
        // Keep HeliOS to telemetry-focused routes to avoid latching onto non-telemetry sockets.
        for port in [5801u16, 5800u16, 80u16] {
            for path in ["/v1/ws/telemetry", "/v1/device/telemetry/ws", "/v1/telemetry/ws"] {
                add_endpoint(port, path);
            }
        }

        let mut unique = HashSet::new();
        return endpoints
            .into_iter()
            .filter(|endpoint| unique.insert(endpoint.clone()))
            .collect();
    }

    for port in [5801u16, 5800u16, 80u16] {
        for path in [
            "/v1/ws/telemetry",
            "/v1/device/ws",
            "/v1/device/telemetry/ws",
            "/v1/telemetry/ws",
            "/ws",
            "/websocket",
            "/websocket_data",
        ] {
            add_endpoint(port, path);
        }
    }

    let mut unique = HashSet::new();
    endpoints
        .into_iter()
        .filter(|endpoint| unique.insert(endpoint.clone()))
        .collect()
}

fn normalize_nonempty_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn summarize_runtime_identity(runtime_info: &HeliosRuntimeInfo) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = runtime_info.os_version.as_deref().map(str::trim) {
        if !value.is_empty() {
            parts.push(format!("OS {value}"));
        }
    }
    if let Some(value) = runtime_info.firmware_version.as_deref().map(str::trim) {
        if !value.is_empty() {
            parts.push(format!("FW {value}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn run_http_telemetry_fallback_loop(
    app: &tauri::AppHandle,
    stream_id: &str,
    ip: &str,
    runtime_product: Option<String>,
) -> bool {
    let source_url = format!("http://{ip} (HTTP telemetry polling)");
    emit_device_telemetry_event(
        app,
        stream_id,
        "connecting",
        "Websocket telemetry unavailable. Switching to HTTP telemetry polling.",
        runtime_product.clone(),
        None,
        None,
        Some(source_url.clone()),
    );

    let mut current_runtime_product = runtime_product;
    let mut current_os_version: Option<String> = None;
    let mut last_summary = String::new();
    let mut last_message_emit = Instant::now() - Duration::from_secs(5);
    let mut last_wait_notice = Instant::now() - Duration::from_secs(20);
    let first_sample_deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_data = false;
    let probe_roborio = current_runtime_product
        .as_deref()
        .map(is_roborio_product)
        .unwrap_or_else(|| is_probable_roborio_ip(ip));

    while telemetry_stream_is_active(stream_id) {
        let runtime_info = if probe_roborio {
            fetch_roborio_runtime_info(ip).or_else(|| fetch_helios_runtime_info(ip))
        } else {
            fetch_helios_runtime_info(ip).or_else(|| {
                if is_probable_roborio_ip(ip) {
                    fetch_roborio_runtime_info(ip)
                } else {
                    None
                }
            })
        };

        if let Some(runtime_info) = runtime_info {
            saw_data = true;

            if current_runtime_product.is_none() && runtime_info.runtime_product.is_some() {
                current_runtime_product = runtime_info.runtime_product.clone();
            }
            if current_os_version.is_none() && runtime_info.os_version.is_some() {
                current_os_version = runtime_info.os_version.clone();
            }

            let summary = normalize_nonempty_text(runtime_info.telemetry_summary.clone())
                .or_else(|| summarize_runtime_identity(&runtime_info));
            if let Some(summary) = summary {
                let should_emit =
                    summary != last_summary || last_message_emit.elapsed() >= Duration::from_secs(2);
                if should_emit {
                    last_summary = summary.clone();
                    last_message_emit = Instant::now();
                    emit_device_telemetry_event(
                        app,
                        stream_id,
                        "data",
                        "Telemetry update received (HTTP polling).",
                        current_runtime_product.clone(),
                        current_os_version.clone(),
                        Some(summary),
                        Some(source_url.clone()),
                    );
                }
            } else if last_wait_notice.elapsed() >= Duration::from_secs(8) {
                emit_device_telemetry_event(
                    app,
                    stream_id,
                    "connecting",
                    "HTTP telemetry endpoint is reachable, waiting for metric payload.",
                    current_runtime_product.clone(),
                    current_os_version.clone(),
                    None,
                    Some(source_url.clone()),
                );
                last_wait_notice = Instant::now();
            }
        } else if !saw_data && Instant::now() >= first_sample_deadline {
            return false;
        } else if last_wait_notice.elapsed() >= Duration::from_secs(8) {
            emit_device_telemetry_event(
                app,
                stream_id,
                "connecting",
                "Waiting for HTTP telemetry response.",
                current_runtime_product.clone(),
                current_os_version.clone(),
                None,
                Some(source_url.clone()),
            );
            last_wait_notice = Instant::now();
        }

        thread::sleep(Duration::from_millis(1400));
    }

    saw_data
}

#[tauri::command]
fn start_device_telemetry_stream(
    app: tauri::AppHandle,
    request: DeviceTelemetryStreamRequest,
) -> Result<String, String> {
    let stream_id = request.stream_id.trim().to_string();
    if stream_id.is_empty() {
        return Err("Missing telemetry stream id.".to_string());
    }
    let ip = request.ip_address.trim().to_string();
    if ip.is_empty() {
        return Err("Missing device IP address for telemetry stream.".to_string());
    }
    let runtime_product = request
        .runtime_product
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    set_active_telemetry_stream_id(Some(stream_id.clone()));
    let app_handle = app.clone();
    thread::spawn(move || {
        emit_device_telemetry_event(
            &app_handle,
            &stream_id,
            "connecting",
            "Connecting to telemetry websocket.",
            runtime_product.clone(),
            None,
            None,
            None,
        );

        let mut connected_socket: Option<(WebSocket<MaybeTlsStream<TcpStream>>, String)> = None;
        let mut connect_errors: Vec<String> = Vec::new();
        for endpoint in build_telemetry_stream_endpoints(&ip, runtime_product.as_deref()) {
            if !telemetry_stream_is_active(&stream_id) {
                return;
            }
            match tungstenite::connect(&endpoint) {
                Ok((mut socket, _)) => {
                    set_websocket_stream_timeouts(&mut socket, Duration::from_millis(800));
                    connected_socket = Some((socket, endpoint));
                    break;
                }
                Err(error) => {
                    if connect_errors.len() < 6 {
                        connect_errors.push(format!("{endpoint} ({error})"));
                    }
                }
            }
        }

        let Some((mut socket, source_url)) = connected_socket else {
            if telemetry_stream_is_active(&stream_id) {
                let websocket_error_detail = if connect_errors.is_empty() {
                    "Unable to connect to device telemetry websocket.".to_string()
                } else {
                    format!(
                        "Unable to connect to device telemetry websocket. Tried: {}",
                        connect_errors.join(" | ")
                    )
                };
                let allow_http_fallback = runtime_product
                    .as_deref()
                    .map(is_roborio_product)
                    .unwrap_or_else(|| is_probable_roborio_ip(&ip));
                if allow_http_fallback {
                    let fallback_success = run_http_telemetry_fallback_loop(
                        &app_handle,
                        &stream_id,
                        &ip,
                        runtime_product.clone(),
                    );
                    if !fallback_success {
                        emit_device_telemetry_event(
                            &app_handle,
                            &stream_id,
                            "error",
                            format!(
                                "{websocket_error_detail} HTTP telemetry fallback did not return data within 10s."
                            ),
                            runtime_product.clone(),
                            None,
                            None,
                            None,
                        );
                        set_active_telemetry_stream_id(None);
                    }
                } else {
                    emit_device_telemetry_event(
                        &app_handle,
                        &stream_id,
                        "error",
                        websocket_error_detail,
                        runtime_product.clone(),
                        None,
                        None,
                        None,
                    );
                    set_active_telemetry_stream_id(None);
                }
            }
            return;
        };

        let mut last_summary = String::new();
        let mut last_message_emit = Instant::now() - Duration::from_secs(5);
        let mut current_runtime_product = runtime_product.clone();
        let mut current_os_version: Option<String> = None;

        loop {
            if !telemetry_stream_is_active(&stream_id) {
                break;
            }

            let message = match socket.read() {
                Ok(message) => message,
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    emit_device_telemetry_event(
                        &app_handle,
                        &stream_id,
                        "error",
                        "Telemetry websocket disconnected.",
                        current_runtime_product.clone(),
                        current_os_version.clone(),
                        None,
                        Some(source_url.clone()),
                    );
                    break;
                }
                Err(error) => {
                    emit_device_telemetry_event(
                        &app_handle,
                        &stream_id,
                        "error",
                        format!("Telemetry websocket error: {error}"),
                        current_runtime_product.clone(),
                        current_os_version.clone(),
                        None,
                        Some(source_url.clone()),
                    );
                    break;
                }
            };

            if let Some(payload) = parse_websocket_payload(&message) {
                let mut runtime_info = extract_helios_runtime_info(&payload);
                merge_photonvision_runtime_info_from_payload(&payload, &mut runtime_info);

                if current_runtime_product.is_none() && runtime_info.runtime_product.is_some() {
                    current_runtime_product = runtime_info.runtime_product.clone();
                }
                if current_os_version.is_none() && runtime_info.os_version.is_some() {
                    current_os_version = runtime_info.os_version.clone();
                }

                if let Some(summary) = normalize_nonempty_text(runtime_info.telemetry_summary.clone()) {
                    let summary_segment_count = summary
                        .split('·')
                        .map(str::trim)
                        .filter(|segment| !segment.is_empty())
                        .count();
                    let expects_structured_helios_payload = current_runtime_product
                        .as_deref()
                        .map(str::to_ascii_lowercase)
                        .map(|product| {
                            product.contains("helios") || product.contains("prometheus")
                        })
                        .unwrap_or(false);
                    let structured_payload = payload_contains_structured_helios_metrics(&payload);

                    if expects_structured_helios_payload && !structured_payload {
                        continue;
                    }
                    if !structured_payload && summary_segment_count < 2 {
                        continue;
                    }

                    let should_emit = summary != last_summary
                        || last_message_emit.elapsed() >= Duration::from_secs(2);
                    if should_emit {
                        last_summary = summary.clone();
                        last_message_emit = Instant::now();
                        emit_device_telemetry_event(
                            &app_handle,
                            &stream_id,
                            "data",
                            "Telemetry frame received.",
                            current_runtime_product.clone(),
                            current_os_version.clone(),
                            Some(summary),
                            Some(source_url.clone()),
                        );
                    }
                }
            }

            if let Message::Ping(ping_data) = message {
                let _ = socket.send(Message::Pong(ping_data));
            }
        }

        let _ = socket.close(None);
        if telemetry_stream_is_active(&stream_id) {
            set_active_telemetry_stream_id(None);
            emit_device_telemetry_event(
                &app_handle,
                &stream_id,
                "stopped",
                "Telemetry stream stopped.",
                current_runtime_product,
                current_os_version,
                None,
                Some(source_url),
            );
        }
    });

    Ok("Telemetry stream started in background.".to_string())
}

#[tauri::command]
fn stop_device_telemetry_stream(request: DeviceTelemetryStopRequest) -> Result<String, String> {
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
async fn get_connection_status() -> Result<ConnectionStatusSnapshot, String> {
    tauri::async_runtime::spawn_blocking(get_connection_status_blocking)
        .await
        .map_err(|error| format!("Connection status worker failed: {error}"))?
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

fn is_active_connection_neighbor_state(state: Option<&str>) -> bool {
    matches!(
        state.map(|value| value.to_ascii_uppercase()),
        Some(state) if matches!(state.as_str(), "REACHABLE" | "DELAY" | "PROBE" | "PERMANENT" | "STALE")
    )
}

#[tauri::command]
async fn fetch_device_log(request: DeviceLogRequest) -> Result<DeviceLogResult, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_device_log_blocking(request))
        .await
        .map_err(|error| format!("Device log worker failed: {error}"))?
}

fn fetch_device_log_blocking(request: DeviceLogRequest) -> Result<DeviceLogResult, String> {
    let ip = request.ip_address.trim();
    if ip.is_empty() {
        return Err("Missing device IP address.".to_string());
    }

    let max_lines = request.max_lines.unwrap_or(500).clamp(100, 4000);
    let runtime_product = request
        .runtime_product
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if runtime_product
        .map(is_photonvision_product)
        .unwrap_or(false)
    {
        let websocket_logs = fetch_photonvision_client_logs(ip, max_lines);
        let websocket_line_count = websocket_logs
            .as_ref()
            .map(|(_, line_count, _)| *line_count)
            .unwrap_or(0);
        let archived_logs = if websocket_line_count < 8 {
            fetch_photonvision_archived_logs(ip, max_lines)
        } else {
            None
        };

        if let Some((websocket_text, _, _)) = websocket_logs.as_ref() {
            if let Some((archive_text, archive_line_count, archive_truncated, archive_source_url)) =
                archived_logs.as_ref()
            {
                let (merged_text, merged_line_count, merged_truncated) =
                    merge_log_texts(archive_text, websocket_text, max_lines);
                return Ok(DeviceLogResult {
                    success: true,
                    source_url: Some(format!(
                        "ws://{ip}:5800/websocket_data + {archive_source_url}"
                    )),
                    line_count: merged_line_count,
                    truncated: merged_truncated || *archive_truncated,
                    fetched_at_epoch_ms: epoch_ms(),
                    log_text: merged_text,
                    message: format!(
                        "PhotonVision client logs loaded (live websocket + {} archived lines).",
                        archive_line_count
                    ),
                });
            }

            let (websocket_text, line_count, truncated) = websocket_logs.unwrap();
            return Ok(DeviceLogResult {
                success: true,
                source_url: Some(format!("ws://{ip}:5800/websocket_data")),
                line_count,
                truncated,
                fetched_at_epoch_ms: epoch_ms(),
                log_text: websocket_text,
                message: "PhotonVision client logs loaded from websocket.".to_string(),
            });
        }

        if let Some((archive_text, line_count, truncated, archive_source_url)) = archived_logs {
            return Ok(DeviceLogResult {
                success: true,
                source_url: Some(archive_source_url),
                line_count,
                truncated,
                fetched_at_epoch_ms: epoch_ms(),
                log_text: archive_text,
                message: "PhotonVision client logs loaded from archived log files.".to_string(),
            });
        }

        return Ok(DeviceLogResult {
            success: false,
            source_url: Some(format!("ws://{ip}:5800/websocket_data")),
            line_count: 0,
            truncated: false,
            fetched_at_epoch_ms: epoch_ms(),
            log_text: String::new(),
            message: "No PhotonVision client logs received from websocket.".to_string(),
        });
    }

    let probe_targets = build_log_probe_targets();
    if probe_targets.is_empty() {
        return Ok(DeviceLogResult {
            success: false,
            source_url: None,
            line_count: 0,
            truncated: false,
            fetched_at_epoch_ms: epoch_ms(),
            log_text: String::new(),
            message: "No known log endpoints for this device type.".to_string(),
        });
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(3200))
        .build()
        .map_err(|error| format!("Unable to build log HTTP client: {error}"))?;

    for (port, path) in probe_targets {
        let url = format!("http://{ip}:{port}{path}");
        let response = match client
            .get(&url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .header(
                reqwest::header::ACCEPT,
                "application/json,text/plain,application/zip,*/*",
            )
            .send()
        {
            Ok(response) => response,
            Err(_) => continue,
        };

        if !response.status().is_success() {
            continue;
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let payload = match response.bytes() {
            Ok(body) => body,
            Err(_) => continue,
        };

        let Some(raw_log_text) = extract_log_text_from_response(&payload, content_type.as_deref())
        else {
            continue;
        };
        let (log_text, line_count, truncated) = constrain_log_lines(&raw_log_text, max_lines);
        if log_text.trim().is_empty() {
            continue;
        }

        return Ok(DeviceLogResult {
            success: true,
            source_url: Some(url),
            line_count,
            truncated,
            fetched_at_epoch_ms: epoch_ms(),
            log_text,
            message: "Device logs loaded.".to_string(),
        });
    }

    Ok(DeviceLogResult {
        success: false,
        source_url: None,
        line_count: 0,
        truncated: false,
        fetched_at_epoch_ms: epoch_ms(),
        log_text: String::new(),
        message: "No log endpoint responded for this device.".to_string(),
    })
}

#[tauri::command]
async fn list_roborio_wpilib_logs(
    request: WpilibLogListRequest,
) -> Result<WpilibLogListResult, String> {
    tauri::async_runtime::spawn_blocking(move || list_roborio_wpilib_logs_blocking(request))
        .await
        .map_err(|error| format!("WPILib log listing worker failed: {error}"))?
}

fn list_roborio_wpilib_logs_blocking(
    request: WpilibLogListRequest,
) -> Result<WpilibLogListResult, String> {
    let host = normalize_roborio_host(&request.ip_address)?;
    let list_output = run_roborio_ssh_command(
        &host,
        "sh -c 'for d in /home/lvuser/logs /u/logs; do [ -d \"$d\" ] || continue; for f in \"$d\"/*.wpilog; do [ -f \"$f\" ] || continue; size=$(wc -c < \"$f\" 2>/dev/null || echo 0); mtime=$(stat -c %Y \"$f\" 2>/dev/null || busybox stat -c %Y \"$f\" 2>/dev/null || echo 0); base=$(basename \"$f\"); printf \"%s|%s|%s|%s\\n\" \"$d\" \"$base\" \"$size\" \"$mtime\"; done; done'",
        Duration::from_secs(10),
    )?;
    let stdout = String::from_utf8_lossy(&list_output.stdout);
    let mut entries = stdout
        .lines()
        .filter_map(parse_wpilib_log_list_line)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| Reverse(entry.modified_epoch_ms.unwrap_or(0)));
    entries.dedup_by(|left, right| left.remote_path == right.remote_path);

    let message = if entries.is_empty() {
        "No WPILib logs found on roboRIO.".to_string()
    } else {
        format!("Found {} WPILib log file(s) on roboRIO.", entries.len())
    };

    Ok(WpilibLogListResult {
        success: true,
        fetched_at_epoch_ms: epoch_ms(),
        entries,
        message,
    })
}

#[tauri::command]
async fn download_roborio_wpilib_logs(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        perform_roborio_wpilib_download(&request.ip_address, &request.selected_logs, request.download_directory)
    })
    .await
    .map_err(|error| format!("WPILib log download worker failed: {error}"))?
}

#[tauri::command]
async fn delete_roborio_wpilib_logs(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        perform_roborio_wpilib_delete(&request.ip_address, &request.selected_logs)
    })
    .await
    .map_err(|error| format!("WPILib log delete worker failed: {error}"))?
}

#[tauri::command]
async fn download_and_delete_roborio_wpilib_logs(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let download_result = perform_roborio_wpilib_download(
            &request.ip_address,
            &request.selected_logs,
            request.download_directory.clone(),
        )?;
        let downloaded_paths = download_result
            .results
            .iter()
            .filter(|item| item.success)
            .map(|item| item.remote_path.clone())
            .collect::<HashSet<_>>();
        let selected_for_delete = request
            .selected_logs
            .iter()
            .filter(|entry| downloaded_paths.contains(&entry.remote_path))
            .cloned()
            .collect::<Vec<_>>();
        if selected_for_delete.is_empty() {
            return Ok(download_result);
        }

        let delete_result = perform_roborio_wpilib_delete(&request.ip_address, &selected_for_delete)?;
        let mut merged_items = Vec::new();
        let mut success_count = 0usize;
        for download_item in &download_result.results {
            let delete_item = delete_result
                .results
                .iter()
                .find(|candidate| candidate.remote_path == download_item.remote_path);
            let (success, message) = if let Some(delete_item) = delete_item {
                (
                    download_item.success && delete_item.success,
                    format!("{} {}", download_item.message, delete_item.message),
                )
            } else {
                (download_item.success, download_item.message.clone())
            };
            if success {
                success_count += 1;
            }
            merged_items.push(WpilibLogActionItemResult {
                file_name: download_item.file_name.clone(),
                remote_path: download_item.remote_path.clone(),
                local_path: download_item.local_path.clone(),
                success,
                message,
            });
        }

        let processed_count = merged_items.len();
        let failed_count = processed_count.saturating_sub(success_count);
        Ok(WpilibLogActionResult {
            success: failed_count == 0,
            processed_count,
            success_count,
            failed_count,
            completed_at_epoch_ms: epoch_ms(),
            download_directory: download_result.download_directory.clone(),
            results: merged_items,
            message: if failed_count == 0 {
                format!(
                    "Downloaded and deleted {} WPILib log file(s).",
                    processed_count
                )
            } else {
                format!(
                    "Downloaded and deleted with {} failure(s). Check per-file results.",
                    failed_count
                )
            },
        })
    })
    .await
    .map_err(|error| format!("WPILib log download+delete worker failed: {error}"))?
}

fn perform_roborio_wpilib_download(
    ip_address: &str,
    selected_logs: &[WpilibLogSelection],
    download_directory: Option<String>,
) -> Result<WpilibLogActionResult, String> {
    let host = normalize_roborio_host(ip_address)?;
    let entries = normalize_wpilib_log_selections(selected_logs)?;
    if entries.is_empty() {
        return Err("No WPILib log files were selected.".to_string());
    }
    let download_root = resolve_download_directory(download_directory)?;
    fs::create_dir_all(&download_root).map_err(|error| {
        format!(
            "Unable to create download directory '{}': {error}",
            download_root.to_string_lossy()
        )
    })?;

    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let local_path = unique_download_destination(&download_root, &entry.file_name);
        let local_path_text = local_path.to_string_lossy().to_string();
        match run_roborio_scp_command(
            &host,
            &entry.remote_path,
            &local_path_text,
            Duration::from_secs(12),
        ) {
            Ok(()) => {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: Some(local_path_text),
                    success: true,
                    message: "Downloaded.".to_string(),
                });
            }
            Err(error) => {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: None,
                    success: false,
                    message: error,
                });
            }
        }
    }

    let success_count = results.iter().filter(|item| item.success).count();
    let processed_count = results.len();
    let failed_count = processed_count.saturating_sub(success_count);
    Ok(WpilibLogActionResult {
        success: failed_count == 0,
        processed_count,
        success_count,
        failed_count,
        completed_at_epoch_ms: epoch_ms(),
        download_directory: Some(download_root.to_string_lossy().to_string()),
        results,
        message: if failed_count == 0 {
            format!("Downloaded {} WPILib log file(s).", processed_count)
        } else {
            format!(
                "Downloaded {} file(s) with {} failure(s).",
                success_count, failed_count
            )
        },
    })
}

fn perform_roborio_wpilib_delete(
    ip_address: &str,
    selected_logs: &[WpilibLogSelection],
) -> Result<WpilibLogActionResult, String> {
    let host = normalize_roborio_host(ip_address)?;
    let entries = normalize_wpilib_log_selections(selected_logs)?;
    if entries.is_empty() {
        return Err("No WPILib log files were selected.".to_string());
    }

    let mut quoted_paths = Vec::with_capacity(entries.len());
    for entry in &entries {
        quoted_paths.push(shell_quote_remote_path(&entry.remote_path));
    }
    let delete_command = format!("sh -c 'rm -f -- {}'", quoted_paths.join(" "));
    let delete_result = run_roborio_ssh_command(&host, &delete_command, Duration::from_secs(10));

    let mut results = Vec::with_capacity(entries.len());
    match delete_result {
        Ok(_) => {
            for entry in entries {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: None,
                    success: true,
                    message: "Deleted.".to_string(),
                });
            }
        }
        Err(error) => {
            for entry in entries {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: None,
                    success: false,
                    message: format!("Delete failed: {error}"),
                });
            }
        }
    }

    let success_count = results.iter().filter(|item| item.success).count();
    let processed_count = results.len();
    let failed_count = processed_count.saturating_sub(success_count);
    Ok(WpilibLogActionResult {
        success: failed_count == 0,
        processed_count,
        success_count,
        failed_count,
        completed_at_epoch_ms: epoch_ms(),
        download_directory: None,
        results,
        message: if failed_count == 0 {
            format!("Deleted {} WPILib log file(s).", processed_count)
        } else {
            format!("Delete failed for {} file(s).", failed_count)
        },
    })
}

fn run_roborio_ssh_command(
    host: &str,
    remote_command: &str,
    timeout: Duration,
) -> Result<Output, String> {
    let target = format!("lvuser@{host}");
    let mut last_error = "SSH command failed for roboRIO.".to_string();
    for option_profile in roborio_ssh_option_profiles() {
        let mut args = option_profile;
        args.push("-T".to_string());
        args.push(target.clone());
        args.push(remote_command.to_string());
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command_with_timeout("ssh", &arg_refs, timeout)?;
        if result.timed_out {
            return Err("SSH command timed out while communicating with roboRIO.".to_string());
        }
        if result.output.status.success() {
            return Ok(result.output);
        }

        let stderr = String::from_utf8_lossy(&result.output.stderr).trim().to_string();
        last_error = if stderr.is_empty() {
            "SSH command failed for roboRIO.".to_string()
        } else {
            format!("SSH command failed for roboRIO: {stderr}")
        };
        if !should_retry_roborio_ssh_with_legacy_options(&stderr) {
            break;
        }
    }

    Err(last_error)
}

fn normalize_roborio_host(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Missing roboRIO host/IP address.".to_string());
    }
    let valid = trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':'));
    if !valid {
        return Err("Invalid roboRIO host/IP format.".to_string());
    }
    Ok(trimmed.to_string())
}

fn run_roborio_scp_command(
    host: &str,
    remote_path: &str,
    local_path: &str,
    timeout: Duration,
) -> Result<(), String> {
    let remote_spec = format!("lvuser@{host}:{remote_path}");
    let mut last_error = "Download failed.".to_string();

    for option_profile in roborio_scp_option_profiles() {
        let mut args = vec!["-q".to_string()];
        args.extend(option_profile);
        args.push(remote_spec.clone());
        args.push(local_path.to_string());
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command_with_timeout("scp", &arg_refs, timeout)?;
        if result.timed_out {
            return Err("Download timed out.".to_string());
        }
        if result.output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&result.output.stderr).trim().to_string();
        last_error = if stderr.is_empty() {
            "Download failed.".to_string()
        } else {
            format!("Download failed: {stderr}")
        };
        if !should_retry_roborio_ssh_with_legacy_options(&stderr) {
            break;
        }
    }

    Err(last_error)
}

fn roborio_ssh_option_profiles() -> Vec<Vec<String>> {
    let known_hosts_sink = roborio_known_hosts_sink().to_string();
    vec![
        vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "NumberOfPasswordPrompts=0".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=accept-new".to_string(),
            "-o".to_string(),
            "ConnectTimeout=4".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=2".to_string(),
            "-o".to_string(),
            "ServerAliveInterval=2".to_string(),
            "-o".to_string(),
            "ServerAliveCountMax=2".to_string(),
        ],
        vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "NumberOfPasswordPrompts=0".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            format!("UserKnownHostsFile={known_hosts_sink}"),
            "-o".to_string(),
            "ConnectTimeout=4".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=2".to_string(),
        ],
        vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "NumberOfPasswordPrompts=0".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            format!("UserKnownHostsFile={known_hosts_sink}"),
            "-o".to_string(),
            "HostKeyAlgorithms=+ssh-rsa".to_string(),
            "-o".to_string(),
            "PubkeyAcceptedAlgorithms=+ssh-rsa".to_string(),
            "-o".to_string(),
            "ConnectTimeout=4".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=2".to_string(),
        ],
    ]
}

fn roborio_scp_option_profiles() -> Vec<Vec<String>> {
    roborio_ssh_option_profiles()
}

fn roborio_known_hosts_sink() -> &'static str {
    if cfg!(target_os = "windows") {
        "NUL"
    } else {
        "/dev/null"
    }
}

fn should_retry_roborio_ssh_with_legacy_options(stderr: &str) -> bool {
    let normalized = stderr.to_ascii_lowercase();
    normalized.contains("host key verification failed")
        || normalized.contains("remote host identification has changed")
        || normalized.contains("no matching host key type found")
        || normalized.contains("unable to negotiate with")
        || normalized.contains("bad configuration option")
        || normalized.contains("key exchange")
}

fn parse_wpilib_log_list_line(line: &str) -> Option<WpilibLogEntry> {
    let mut parts = line.split('|');
    let directory = parts.next()?.trim();
    let file_name = parts.next()?.trim();
    let size_bytes = parts.next()?.trim().parse::<u64>().ok()?;
    let modified_epoch_ms = parts
        .next()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000));

    if !WPILIB_ROBORIO_LOG_DIRS.contains(&directory) || !is_safe_wpilib_log_filename(file_name) {
        return None;
    }
    let remote_path = format!("{directory}/{file_name}");
    let id = format!(
        "{}:{}",
        directory.trim_start_matches('/').replace('/', "-"),
        file_name
    );
    Some(WpilibLogEntry {
        id,
        file_name: file_name.to_string(),
        remote_path,
        size_bytes,
        modified_epoch_ms,
    })
}

fn normalize_wpilib_log_selections(
    selections: &[WpilibLogSelection],
) -> Result<Vec<WpilibLogSelection>, String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for selection in selections {
        let file_name = selection.file_name.trim();
        let remote_path = selection.remote_path.trim();
        if !is_safe_wpilib_log_filename(file_name) {
            continue;
        }
        let Some(clean_remote_path) = normalize_safe_wpilib_remote_path(remote_path) else {
            continue;
        };
        if !clean_remote_path.ends_with(file_name) {
            continue;
        }
        if !seen.insert(clean_remote_path.clone()) {
            continue;
        }
        normalized.push(WpilibLogSelection {
            file_name: file_name.to_string(),
            remote_path: clean_remote_path,
        });
    }
    if normalized.is_empty() {
        return Err("No valid WPILib log selections were provided.".to_string());
    }
    Ok(normalized)
}

fn normalize_safe_wpilib_remote_path(remote_path: &str) -> Option<String> {
    let trimmed = remote_path.trim();
    if trimmed.contains("..") || trimmed.contains('\\') || trimmed.contains('\0') {
        return None;
    }
    for directory in WPILIB_ROBORIO_LOG_DIRS {
        let prefix = format!("{directory}/");
        if !trimmed.starts_with(&prefix) {
            continue;
        }
        let file_name = &trimmed[prefix.len()..];
        if is_safe_wpilib_log_filename(file_name) {
            return Some(format!("{directory}/{file_name}"));
        }
    }
    None
}

fn is_safe_wpilib_log_filename(file_name: &str) -> bool {
    let trimmed = file_name.trim();
    if trimmed.is_empty() || trimmed.len() > 180 || !trimmed.ends_with(".wpilog") {
        return false;
    }
    trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

fn resolve_download_directory(download_directory: Option<String>) -> Result<PathBuf, String> {
    let Some(directory) = download_directory else {
        return Err("Missing download directory for WPILib logs.".to_string());
    };
    let trimmed = directory.trim();
    if trimmed.is_empty() {
        return Err("Download directory cannot be empty.".to_string());
    }
    Ok(PathBuf::from(trimmed))
}

fn unique_download_destination(directory: &Path, file_name: &str) -> PathBuf {
    let mut candidate = directory.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("log");
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wpilog");

    for index in 1..5000 {
        let replacement = directory.join(format!("{stem}_{index}.{extension}"));
        if !replacement.exists() {
            return replacement;
        }
        candidate = replacement;
    }

    candidate
}

fn shell_quote_remote_path(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn normalize_ota_probe_targets(raw_targets: &[String]) -> Vec<String> {
    let mut targets = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for raw in raw_targets {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        targets.push(trimmed.to_string());
    }
    targets
}

fn ota_runtime_state_is_attachable(state: &OtaRuntimeState) -> bool {
    let stage = state
        .stage
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("idle")
        .to_ascii_lowercase();
    if stage == "idle" {
        return false;
    }
    if is_ota_terminal_error_stage(&stage) {
        return false;
    }
    if is_ota_apply_stage(&stage) {
        return true;
    }
    state
        .progress_percent
        .map(|value| value > 0.0)
        .unwrap_or(false)
        || state
            .update_id
            .as_deref()
            .map(str::trim)
            .map(|value| !value.is_empty())
            .unwrap_or(false)
}

#[tauri::command]
fn list_helios_release_images() -> Result<Vec<ReleaseImageOption>, String> {
    fetch_release_images_from_github()
}

#[tauri::command]
fn get_host_setup_status() -> Result<HostSetupStatus, String> {
    Ok(build_host_setup_status())
}

#[tauri::command]
fn run_host_setup_repair() -> Result<HostSetupRepairResult, String> {
    run_host_setup_repair_internal()
}

#[tauri::command]
fn relaunch_elevated() -> Result<OperationResult, String> {
    relaunch_elevated_internal()
}

#[tauri::command]
fn probe_existing_device_ota_update(
    request: ProbeExistingOtaUpdateRequest,
) -> Result<Option<ExistingOtaUpdateProbeResult>, String> {
    let targets = normalize_ota_probe_targets(&request.target_ip_addresses);
    if targets.is_empty() {
        return Ok(None);
    }

    let timeout_ms = request.timeout_ms.unwrap_or(1500).clamp(400, 5000);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| format!("Unable to build OTA probe HTTP client: {error}"))?;

    for target_ip in targets {
        let api_bases = ota_api_base_urls(&target_ip);
        if api_bases.is_empty() {
            continue;
        }

        let state = match fetch_ota_runtime_state(&client, &api_bases) {
            Ok(state) => state,
            Err(_) => continue,
        };
        if !ota_runtime_state_is_attachable(&state) {
            continue;
        }

        let stage = state
            .stage
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let progress_percent = state.progress_percent.map(|value| value.clamp(0.0, 100.0));
        let update_id = state
            .update_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let last_error = state
            .last_error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        return Ok(Some(ExistingOtaUpdateProbeResult {
            target_ip_address: target_ip,
            update_id,
            stage,
            progress_percent,
            last_error,
        }));
    }

    Ok(None)
}

#[tauri::command]
fn attach_existing_ota_update(
    app: tauri::AppHandle,
    request: AttachExistingOtaUpdateRequest,
) -> Result<String, String> {
    let run_id = request.run_id.trim().to_string();
    if run_id.is_empty() {
        return Err("Missing run id for OTA attach workflow.".to_string());
    }
    let target_ip = request.target_ip_address.trim().to_string();
    if target_ip.is_empty() {
        return Err("Missing device IP for OTA attach workflow.".to_string());
    }

    let api_bases = ota_api_base_urls(&target_ip);
    if api_bases.is_empty() {
        return Err(format!(
            "Unable to build OTA API endpoints for target '{target_ip}'."
        ));
    }

    let probe_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1800))
        .build()
        .map_err(|error| format!("Unable to build OTA attach probe client: {error}"))?;
    let state = fetch_ota_runtime_state(&probe_client, &api_bases)
        .map_err(|error| format!("Unable to query OTA state on {target_ip}: {error}"))?;
    if !ota_runtime_state_is_attachable(&state) {
        let stage = state
            .stage
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("idle");
        return Err(format!(
            "Device at {target_ip} does not report an active OTA update (stage: {stage})."
        ));
    }

    begin_updater_job()?;
    let timeout_seconds = request.timeout_seconds.unwrap_or(900).clamp(120, 1800);
    let expected_update_id = request
        .expected_update_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            state
                .update_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    persist_ota_recovery_start(
        &app,
        Some(run_id.as_str()),
        Some(target_ip.as_str()),
        Some(timeout_seconds),
    );
    persist_ota_recovery_update(
        &app,
        Some(run_id.as_str()),
        expected_update_id.clone(),
        Some("Attached to existing OTA update session.".to_string()),
    );

    let app_handle = app.clone();
    let target_ip_for_thread = target_ip.clone();
    let expected_update_id_for_thread = expected_update_id.clone();
    thread::spawn(move || {
        let run_id_ref = Some(run_id.as_str());
        emit_updater_progress(
            &app_handle,
            run_id_ref,
            "ota",
            "info",
            "info",
            format!(
                "Attached to in-progress OTA update on {target_ip_for_thread}. Monitoring live OTA state."
            ),
        );

        let api_bases = ota_api_base_urls(&target_ip_for_thread);
        let result = wait_for_ota_reboot_and_reconnect(
            &app_handle,
            run_id_ref,
            &target_ip_for_thread,
            &api_bases,
            timeout_seconds,
            expected_update_id_for_thread.as_deref(),
        );
        match result {
            Ok(message) => {
                emit_updater_progress(
                    &app_handle,
                    run_id_ref,
                    "ota",
                    "complete",
                    "success",
                    message,
                );
            }
            Err(error) => {
                emit_updater_progress(
                    &app_handle,
                    run_id_ref,
                    "ota",
                    "complete",
                    "error",
                    format!("Attached OTA monitor failed: {error}"),
                );
            }
        }
        clear_ota_recovery_if_run_matches(&app_handle, run_id_ref);
        finish_updater_job();
    });

    Ok(format!(
        "Attached to existing OTA update on {target_ip}. Monitoring started."
    ))
}

#[tauri::command]
fn cancel_helios_update(
    app: tauri::AppHandle,
    request: CancelUpdateRequest,
) -> Result<String, String> {
    if !UPDATER_JOB_ACTIVE.load(Ordering::SeqCst) {
        return Ok("No active updater operation is running.".to_string());
    }
    set_update_cancel_requested(true);
    let mode = request.mode.as_deref().unwrap_or("flash");
    emit_updater_progress(
        &app,
        request.run_id.as_deref(),
        mode,
        "complete",
        "error",
        "Cancel requested by user. Waiting for the current operation to stop.",
    );
    Ok("Cancel request accepted. Stopping current updater operation.".to_string())
}

#[tauri::command]
fn recover_ota_update_session(
    app: tauri::AppHandle,
) -> Result<Option<UpdaterRecoverySession>, String> {
    if UPDATER_JOB_ACTIVE.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let Some(state) = load_recoverable_ota_state(&app) else {
        return Ok(None);
    };
    let target_ip = state
        .target_ip_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Recovery state is missing OTA target IP address.".to_string())?
        .to_string();
    begin_updater_job()?;
    let run_id = state.run_id.clone();
    let expected_update_id = state.expected_update_id.clone();
    let started_at_epoch_ms = state.started_at_epoch_ms;
    let timeout_seconds = state.monitor_timeout_seconds.unwrap_or(900).clamp(120, 1800);
    let target_ip_for_thread = target_ip.clone();
    let app_handle = app.clone();
    persist_ota_recovery_update(
        &app_handle,
        Some(run_id.as_str()),
        expected_update_id.clone(),
        Some("Resuming OTA monitor after app restart.".to_string()),
    );
    thread::spawn(move || {
        let run_id_ref = Some(run_id.as_str());
        emit_updater_progress(
            &app_handle,
            run_id_ref,
            "ota",
            "info",
            "info",
            format!(
                "Recovered OTA update session for {target_ip_for_thread}. Resuming reboot/reconnect monitor."
            ),
        );
        let api_bases = ota_api_base_urls(&target_ip_for_thread);
        let result = wait_for_ota_reboot_and_reconnect(
            &app_handle,
            run_id_ref,
            &target_ip_for_thread,
            &api_bases,
            timeout_seconds,
            expected_update_id.as_deref(),
        );
        match result {
            Ok(message) => {
                emit_updater_progress(
                    &app_handle,
                    run_id_ref,
                    "ota",
                    "complete",
                    "success",
                    message,
                );
            }
            Err(error) => {
                emit_updater_progress(
                    &app_handle,
                    run_id_ref,
                    "ota",
                    "complete",
                    "error",
                    format!("Recovered OTA monitor failed: {error}"),
                );
            }
        }
        clear_ota_recovery_if_run_matches(&app_handle, run_id_ref);
        finish_updater_job();
    });

    Ok(Some(UpdaterRecoverySession {
        run_id: state.run_id,
        mode: "ota".to_string(),
        target_ip_address: Some(target_ip),
        expected_update_id: state.expected_update_id,
        started_at_epoch_ms,
        resumed_at_epoch_ms: epoch_ms(),
        note: Some("Recovered OTA updater session after restart.".to_string()),
    }))
}

#[tauri::command]
fn start_install_helios_os(
    app: tauri::AppHandle,
    request: ReleaseInstallRequest,
) -> Result<String, String> {
    begin_updater_job()?;
    let mode = if request.mount_only.unwrap_or(false) {
        "mount".to_string()
    } else if request.prefer_ota.unwrap_or(false) {
        "ota".to_string()
    } else {
        "flash".to_string()
    };
    let run_id = request.run_id.clone();
    if mode == "ota" {
        persist_ota_recovery_start(
            &app,
            run_id.as_deref(),
            request.target_ip_address.as_deref(),
            request.timeout_seconds,
        );
    } else {
        clear_updater_recovery_state(&app);
    }
    let app_handle = app.clone();
    thread::spawn(move || {
        let run_id_ref = run_id.as_deref();
        emit_updater_progress(
            &app_handle,
            run_id_ref,
            &mode,
            "info",
            "info",
            "Updater started in background.",
        );
        let result = install_helios_os(app_handle.clone(), request);
        if let Err(error) = result {
            emit_updater_progress(
                &app_handle,
                run_id_ref,
                &mode,
                "complete",
                "error",
                format!("Update failed: {error}"),
            );
        }
        if mode == "ota" {
            clear_ota_recovery_if_run_matches(&app_handle, run_id_ref);
        }
        finish_updater_job();
    });
    Ok("Updater started in background.".to_string())
}

#[tauri::command]
fn start_mount_helios_bootloader(
    app: tauri::AppHandle,
    request: MountRequest,
) -> Result<String, String> {
    clear_updater_recovery_state(&app);
    begin_updater_job()?;
    let run_id = request.run_id.clone();
    let app_handle = app.clone();
    thread::spawn(move || {
        let run_id_ref = run_id.as_deref();
        emit_updater_progress(
            &app_handle,
            run_id_ref,
            "mount",
            "info",
            "info",
            "Mount workflow started in background.",
        );
        let result = mount_helios_bootloader(app_handle.clone(), request);
        if let Err(error) = result {
            emit_updater_progress(
                &app_handle,
                run_id_ref,
                "mount",
                "complete",
                "error",
                format!("Mount workflow failed: {error}"),
            );
        }
        finish_updater_job();
    });
    Ok("Mount workflow started in background.".to_string())
}
