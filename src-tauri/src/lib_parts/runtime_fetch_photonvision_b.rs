fn fetch_photonvision_client_logs(ip: &str, max_lines: usize) -> Option<(String, usize, bool)> {
    let endpoint = format!("ws://{ip}:5800/websocket_data");
    let (mut socket, _) = tungstenite::connect(endpoint).ok()?;
    set_websocket_stream_timeouts(&mut socket, Duration::from_millis(350));

    let deadline = Instant::now() + Duration::from_millis(2200);
    let mut log_lines: Vec<String> = Vec::new();
    let mut seen_lines = HashSet::new();

    while Instant::now() < deadline {
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
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(_) => break,
        };

        if let Some(payload) = parse_websocket_payload(&message) {
            for line in extract_photonvision_client_log_lines(&payload) {
                if seen_lines.insert(line.clone()) {
                    log_lines.push(line);
                }
            }
        }

        if let Message::Ping(ping_data) = message {
            let _ = socket.send(Message::Pong(ping_data));
        }
    }

    let _ = socket.close(None);

    if log_lines.is_empty() {
        return None;
    }

    let full_text = log_lines.join("\n");
    Some(constrain_log_lines(&full_text, max_lines))
}

fn fetch_photonvision_archived_logs(
    ip: &str,
    max_lines: usize,
) -> Option<(String, usize, bool, String)> {
    let now = epoch_ms();
    if let Ok(cache) = photonvision_archive_log_cache().lock() {
        if let Some(entry) = cache.get(ip) {
            if now.saturating_sub(entry.fetched_at_epoch_ms) <= PHOTONVISION_ARCHIVE_CACHE_TTL_MS {
                let (log_text, line_count, truncated) =
                    constrain_log_lines(&entry.raw_log_text, max_lines);
                return Some((log_text, line_count, truncated, entry.source_url.clone()));
            }
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(4500))
        .build()
        .ok()?;
    let ports = [5800u16, 80u16];
    for port in ports {
        let url = format!("http://{ip}:{port}{PHOTONVISION_ARCHIVE_LOG_ZIP_PATH}");
        let response = match client
            .get(&url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .header(reqwest::header::ACCEPT, "application/zip,*/*")
            .send()
        {
            Ok(response) => response,
            Err(_) => continue,
        };
        if !response.status().is_success() {
            continue;
        }

        let payload = match response.bytes() {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let Some(raw_log_text) = decode_zip_log_payload(&payload) else {
            continue;
        };
        if raw_log_text.trim().is_empty() {
            continue;
        }

        if let Ok(mut cache) = photonvision_archive_log_cache().lock() {
            cache.insert(
                ip.to_string(),
                PhotonvisionArchiveLogCacheEntry {
                    fetched_at_epoch_ms: now,
                    source_url: url.clone(),
                    raw_log_text: raw_log_text.clone(),
                },
            );
        }

        let (log_text, line_count, truncated) = constrain_log_lines(&raw_log_text, max_lines);
        return Some((log_text, line_count, truncated, url));
    }

    None
}

fn merge_log_texts(base_text: &str, append_text: &str, max_lines: usize) -> (String, usize, bool) {
    let mut lines = Vec::new();
    let mut seen = HashSet::new();

    for source in [base_text, append_text] {
        for line in source.lines() {
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let owned = trimmed.to_string();
            if seen.insert(owned.clone()) {
                lines.push(owned);
            }
        }
    }

    let line_count = lines.len();
    let truncated = line_count > max_lines;
    if truncated {
        lines = lines
            .into_iter()
            .rev()
            .take(max_lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    (lines.join("\n"), line_count, truncated)
}

fn extract_photonvision_client_log_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let mut seen = HashSet::new();

    fn push_line(lines: &mut Vec<String>, seen: &mut HashSet<String>, value: Option<String>) {
        let Some(line) = value else {
            return;
        };
        let normalized = line.trim();
        if normalized.is_empty() {
            return;
        }
        let normalized_owned = normalized.to_string();
        if seen.insert(normalized_owned.clone()) {
            lines.push(normalized_owned);
        }
    }

    fn extract_message(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            Value::Object(map) => {
                if let Some(nested) = map.get("logMessage") {
                    if let Some(extracted) = extract_message(nested) {
                        if let Some(level_code) = map.get("logLevel").and_then(Value::as_i64) {
                            let level = match level_code {
                                0 => "ERROR",
                                1 => "WARN",
                                2 => "INFO",
                                3 => "DEBUG",
                                4 => "TRACE",
                                _ => "LOG",
                            };
                            if extracted.contains('[') {
                                return Some(extracted);
                            }
                            return Some(format!("[{level}] {extracted}"));
                        }
                        return Some(extracted);
                    }
                }
                if let Some(message) = map.get("message").and_then(Value::as_str) {
                    let trimmed = message.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
                None
            }
            _ => None,
        }
    }

    if let Value::Object(map) = payload {
        if let Some(log_value) = map.get("log") {
            push_line(&mut lines, &mut seen, extract_message(log_value));
            if let Value::Array(entries) = log_value {
                for entry in entries {
                    push_line(&mut lines, &mut seen, extract_message(entry));
                }
            }
        }

        if let Some(log_message_value) = map.get("logMessage") {
            push_line(&mut lines, &mut seen, extract_message(log_message_value));
        }

        if let Some(logs_value) = map.get("logs") {
            if let Value::Array(entries) = logs_value {
                for entry in entries {
                    push_line(&mut lines, &mut seen, extract_message(entry));
                }
            } else {
                push_line(&mut lines, &mut seen, extract_message(logs_value));
            }
        }
    } else {
        push_line(&mut lines, &mut seen, extract_message(payload));
    }

    lines
}

fn extract_runtime_info_from_plain_text(path: &str, body_text: &str) -> HeliosRuntimeInfo {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in body_text.lines().take(180) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let separator = if trimmed.contains('=') {
            '='
        } else if trimmed.contains(':') {
            ':'
        } else {
            continue;
        };
        if let Some((key, value)) = trimmed.split_once(separator) {
            let key = key.trim().to_ascii_lowercase();
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if !key.is_empty() && !value.is_empty() {
                fields.push((key, value));
            }
        }
    }

    let mut info = HeliosRuntimeInfo::default();
    if !fields.is_empty() {
        info.hostname = normalize_hostname_value(find_scalar_by_patterns(
            &fields,
            &[
                "hostname",
                "host_name",
                "device_hostname",
                "network_hostname",
                "general.hostname",
                "device.hostname",
            ],
        ));
        info.firmware_version = find_scalar_by_patterns(
            &fields,
            &[
                "firmware_version",
                "software_version",
                "app_version",
                "version",
                "build_version",
            ],
        );
        info.os_version = find_scalar_by_patterns(
            &fields,
            &[
                "os_version",
                "pretty_name",
                "image_version",
                "release_version",
                "kernel_version",
            ],
        );
        info.telemetry_summary = build_telemetry_summary(&fields);
    }

    if info.hostname.is_none() && path.to_ascii_lowercase().contains("hostname") {
        info.hostname = normalize_hostname_value(Some(body_text.trim().to_string()));
    }

    let trimmed = body_text.trim();
    if trimmed.contains('.')
        && trimmed.chars().any(|character| character.is_ascii_digit())
        && trimmed.len() <= 64
    {
        let likely_version = trimmed.to_string();
        if info.firmware_version.is_none() && path.contains("version") {
            info.firmware_version = Some(likely_version);
        } else if info.os_version.is_none() && path.contains("release") {
            info.os_version = Some(likely_version);
        }
    }

    info
}
