use super::*;

pub(crate) struct OtaWebsocketMonitor {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    source_url: String,
}

impl OtaWebsocketMonitor {
    pub(crate) fn connect(target_ip: &str) -> Result<Self, String> {
        let mut errors = Vec::new();
        for endpoint in ota_websocket_state_urls(target_ip) {
            match tungstenite::connect(&endpoint) {
                Ok((mut socket, _)) => {
                    set_ota_websocket_stream_timeouts(&mut socket, Duration::from_millis(1200));
                    return Ok(Self {
                        socket,
                        source_url: endpoint,
                    });
                }
                Err(error) => {
                    errors.push(format!("{endpoint} ({error})"));
                }
            }
        }
        if errors.is_empty() {
            Err("No OTA websocket endpoints were available.".to_string())
        } else {
            Err(format!(
                "Unable to connect OTA websocket monitor. Tried: {}",
                errors.join(" | ")
            ))
        }
    }

    pub(crate) fn source_url(&self) -> &str {
        &self.source_url
    }

    pub(crate) fn poll_runtime_state(&mut self) -> Result<Option<OtaRuntimeState>, String> {
        loop {
            let message = match self.socket.read() {
                Ok(message) => message,
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Err("OTA websocket stream disconnected.".to_string());
                }
                Err(error) => {
                    return Err(format!("OTA websocket stream error: {error}"));
                }
            };

            match message {
                Message::Ping(payload) => {
                    let _ = self.socket.send(Message::Pong(payload));
                    continue;
                }
                Message::Pong(_) => continue,
                Message::Close(_) => return Err("OTA websocket stream closed by peer.".to_string()),
                _ => {}
            }

            let Some(payload) = parse_ota_websocket_payload(&message) else {
                continue;
            };
            match parse_ota_websocket_runtime_state(&payload) {
                Ok(Some(state)) => return Ok(Some(state)),
                Ok(None) => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

pub(crate) fn ota_websocket_state_urls(target_ip: &str) -> Vec<String> {
    let mut candidates = vec![
        format!("ws://{target_ip}:5801/v1/ws/ota"),
        format!("ws://{target_ip}:5800/v1/ws/ota"),
        format!("ws://{target_ip}:80/v1/ws/ota"),
        format!("ws://{target_ip}/v1/ws/ota"),
    ];
    candidates.retain(|value| !value.trim().is_empty());
    let mut deduped = Vec::new();
    for candidate in candidates {
        if !deduped.contains(&candidate) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn parse_ota_websocket_payload(message: &Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str::<Value>(text).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes).ok(),
        _ => None,
    }
}

fn parse_ota_websocket_runtime_state(payload: &Value) -> Result<Option<OtaRuntimeState>, String> {
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    if let Some(event_type) = event_type {
        return match event_type.as_str() {
            "snapshot" => {
                let state = parse_ota_runtime_state(payload.get("state"));
                if ota_runtime_state_has_data(&state) {
                    Ok(Some(state))
                } else {
                    Ok(None)
                }
            }
            "stage_progress" => Ok(Some(OtaRuntimeState {
                update_id: payload
                    .get("update_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                stage: Some("applying".to_string()),
                progress_percent: payload
                    .get("percent")
                    .and_then(|value| parse_ota_runtime_progress_value(value, "percent"))
                    .map(|value| value.clamp(0.0, 100.0)),
                last_error: None,
            })),
            "stage_complete" => Ok(Some(OtaRuntimeState {
                update_id: payload
                    .get("update_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                stage: Some("finalizing".to_string()),
                progress_percent: Some(100.0),
                last_error: None,
            })),
            "apply_scheduled" => Ok(Some(OtaRuntimeState {
                update_id: payload
                    .get("update_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                stage: Some("awaiting_window".to_string()),
                progress_percent: Some(92.0),
                last_error: None,
            })),
            "apply_complete" => {
                let reboot_required = payload
                    .get("reboot_required")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                Ok(Some(OtaRuntimeState {
                    update_id: payload
                        .get("update_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    stage: Some(if reboot_required {
                        "rebooting".to_string()
                    } else {
                        "complete".to_string()
                    }),
                    progress_percent: Some(100.0),
                    last_error: None,
                }))
            }
            "rollback_triggered" => Ok(Some(OtaRuntimeState {
                update_id: payload
                    .get("update_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                stage: Some("rolled_back".to_string()),
                progress_percent: Some(0.0),
                last_error: payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            })),
            "error" => {
                let message = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| "Unknown OTA websocket error.".to_string());
                Err(format!("Device OTA websocket reported an error: {message}"))
            }
            _ => Ok(None),
        };
    }

    let state = parse_ota_runtime_state(Some(payload));
    if ota_runtime_state_has_data(&state) {
        Ok(Some(state))
    } else {
        Ok(None)
    }
}

fn ota_runtime_state_has_data(state: &OtaRuntimeState) -> bool {
    state.update_id.is_some()
        || state.stage.as_ref().is_some_and(|value| !value.is_empty())
        || state.progress_percent.is_some()
        || state.last_error.as_ref().is_some_and(|value| !value.is_empty())
}

fn set_ota_websocket_stream_timeouts(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
    }
}
