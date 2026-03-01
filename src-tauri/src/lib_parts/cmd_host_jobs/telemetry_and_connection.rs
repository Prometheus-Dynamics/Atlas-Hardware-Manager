use super::*;
use super::roborio_log_commands::fetch_device_log_blocking;

#[path = "telemetry_and_connection/status_and_logs.rs"]
pub(crate) mod status_and_logs;
pub(crate) use status_and_logs::*;


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

pub(crate) fn normalize_nonempty_text(value: Option<String>) -> Option<String> {
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

pub(crate) struct TelemetryStreamService;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TelemetryServiceError {
    #[error("Missing telemetry stream id.")]
    MissingStreamId,
    #[error("Missing device IP address for telemetry stream.")]
    MissingIpAddress,
    #[error("{0}")]
    Operation(String),
    #[error("Telemetry background worker failed: {0}")]
    Worker(String),
}

impl TelemetryStreamService {
    pub(crate) fn start_device_telemetry_stream(
        app: tauri::AppHandle,
        request: DeviceTelemetryStreamRequest,
    ) -> Result<String, TelemetryServiceError> {
        start_device_telemetry_stream_impl(app, request)
    }
}
#[tauri::command]
pub(crate) fn start_device_telemetry_stream(
    app: tauri::AppHandle,
    request: DeviceTelemetryStreamRequest,
) -> Result<String, String> {
    TelemetryStreamService::start_device_telemetry_stream(app, request).map_err(|error| error.to_string())
}

fn start_device_telemetry_stream_impl(
    app: tauri::AppHandle,
    request: DeviceTelemetryStreamRequest,
) -> Result<String, TelemetryServiceError> {
    let stream_id = request.stream_id.trim().to_string();
    if stream_id.is_empty() {
        return Err(TelemetryServiceError::MissingStreamId);
    }
    let ip = request.ip_address.trim().to_string();
    if ip.is_empty() {
        return Err(TelemetryServiceError::MissingIpAddress);
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
