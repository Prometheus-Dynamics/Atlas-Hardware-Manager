fn discover_flash_targets_sysinfo() -> Result<Vec<FlashTarget>, String> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut targets = Vec::new();

    for disk in disks.list() {
        let removable = disk.is_removable();
        if !removable {
            continue;
        }

        let mount_point = disk.mount_point().to_string_lossy().to_string();
        if mount_point.trim().is_empty() {
            continue;
        }

        let name = disk.name().to_string_lossy().to_string();
        targets.push(FlashTarget {
            path: mount_point.clone(),
            name: if name.trim().is_empty() {
                mount_point
            } else {
                name
            },
            size_bytes: disk.total_space(),
            model: String::new(),
            transport: None,
            removable: true,
            hotplug: true,
            mounted: true,
        });
    }

    targets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(targets)
}

fn collect_mountpoints(device: &Value) -> Vec<String> {
    let mut mountpoints = Vec::new();

    if let Some(points) = device.get("mountpoints").and_then(Value::as_array) {
        for point in points {
            if let Some(text) = point.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    mountpoints.push(trimmed.to_string());
                }
            }
        }
    }

    if let Some(children) = device.get("children").and_then(Value::as_array) {
        for child in children {
            if let Some(points) = child.get("mountpoints").and_then(Value::as_array) {
                for point in points {
                    if let Some(text) = point.as_str() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            mountpoints.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }

    mountpoints.sort();
    mountpoints.dedup();
    mountpoints
}

fn to_bool_flag(value: Option<&Value>) -> bool {
    value
        .and_then(|entry| {
            if let Some(flag) = entry.as_bool() {
                return Some(flag);
            }
            if let Some(number) = entry.as_u64() {
                return Some(number > 0);
            }
            if let Some(text) = entry.as_str() {
                return Some(text == "1" || text.eq_ignore_ascii_case("true"));
            }
            None
        })
        .unwrap_or(false)
}

fn discover_firmware_images(root: &Path) -> Result<Vec<FirmwareImage>, String> {
    let mut results = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((current_dir, depth)) = stack.pop() {
        if depth > MAX_SCAN_DEPTH || results.len() >= MAX_IMAGE_RESULTS {
            continue;
        }

        let read_dir = match fs::read_dir(&current_dir) {
            Ok(entries) => entries,
            Err(error) => {
                return Err(format!(
                    "Unable to read directory '{}': {error}",
                    current_dir.to_string_lossy()
                ));
            }
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry
                    .file_name()
                    .to_string_lossy()
                    .to_string()
                    .to_lowercase();
                if should_skip_directory(&name) {
                    continue;
                }
                stack.push((path, depth + 1));
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().to_string();
            if !is_supported_image_name(&file_name) {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };

            results.push(FirmwareImage {
                path: path.to_string_lossy().to_string(),
                file_name,
                size_bytes: metadata.len(),
                modified_epoch_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64),
            });
        }
    }

    results.sort_by_key(|image| Reverse(image.modified_epoch_ms.unwrap_or_default()));
    if results.len() > MAX_IMAGE_RESULTS {
        results.truncate(MAX_IMAGE_RESULTS);
    }

    Ok(results)
}

fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".cache" | "node_modules" | "target" | "build" | "dist"
    )
}

fn is_supported_image_name(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    lower.ends_with(".img")
        || lower.ends_with(".img.xz")
        || lower.ends_with(".wic")
        || lower.ends_with(".wic.xz")
        || lower.ends_with(".zip")
        || lower.ends_with(".iso")
        || lower.ends_with(".upd")
}

fn fetch_helios_runtime_info(ip: &str) -> Option<HeliosRuntimeInfo> {
    if !is_host_port_responsive(ip, &[5800_u16, 5801_u16, 80_u16, 443_u16]) {
        return None;
    }

    let deadline = Instant::now() + Duration::from_millis(2_400);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()
        .ok()?;

    let mut merged = HeliosRuntimeInfo::default();
    'probe: for path in HELIOS_RUNTIME_API_PATHS {
        let ports: &[u16] = &[5800u16, 5801u16, 80u16];
        for &port in ports {
            if Instant::now() >= deadline {
                break 'probe;
            }

            let url = format!("http://{ip}:{port}{path}");
            let response = match client
                .get(url)
                .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
                .header(reqwest::header::ACCEPT, "application/json,text/plain,*/*")
                .send()
            {
                Ok(response) => response,
                Err(_) => continue,
            };

            if !response.status().is_success() {
                continue;
            }

            let server_header = response
                .headers()
                .get(reqwest::header::SERVER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let body_text = match response.text() {
                Ok(text) => text,
                Err(_) => continue,
            };

            let mut runtime_info = match serde_json::from_str::<Value>(&body_text) {
                Ok(payload) => extract_helios_runtime_info(&payload),
                Err(_) => extract_runtime_info_from_plain_text(path, &body_text),
            };
            if runtime_info.runtime_product.is_none() {
                runtime_info.runtime_product = classify_runtime_product_from_response(
                    path,
                    &body_text,
                    server_header.as_deref(),
                );
            }
            if !runtime_info.has_data() {
                continue;
            }

            merged.merge_missing(runtime_info);
            if merged.is_complete() {
                return Some(merged);
            }
            if merged.runtime_product.is_some()
                && (merged.firmware_version.is_some()
                    || merged.os_version.is_some()
                    || merged.telemetry_summary.is_some())
            {
                break 'probe;
            }
        }
    }

    maybe_enrich_with_photonvision_websocket(ip, &mut merged);

    if merged.telemetry_summary.is_some()
        || merged.firmware_version.is_some()
        || merged.os_version.is_some()
    {
        Some(merged)
    } else {
        None
    }
}

fn fetch_roborio_runtime_info(ip: &str) -> Option<HeliosRuntimeInfo> {
    if !is_host_port_responsive(ip, &ROBORIO_RUNTIME_TCP_PORTS) {
        return None;
    }

    let deadline = Instant::now() + Duration::from_millis(2_300);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(650))
        .build()
        .ok()?;

    let mut merged = HeliosRuntimeInfo {
        runtime_product: Some("roboRIO".to_string()),
        ..HeliosRuntimeInfo::default()
    };
    'probe: for path in ROBORIO_RUNTIME_API_PATHS {
        let ports: &[u16] = if path.starts_with("/?action=") {
            &[1250u16, 80u16, 3580u16]
        } else {
            &[80u16, 3580u16, 1250u16]
        };
        for &port in ports {
            if Instant::now() >= deadline {
                break 'probe;
            }

            let url = format!("http://{ip}:{port}{path}");
            let response = match client
                .get(url)
                .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
                .header(reqwest::header::ACCEPT, "application/json,text/plain,text/html,*/*")
                .send()
            {
                Ok(response) => response,
                Err(_) => continue,
            };

            if !response.status().is_success() {
                continue;
            }

            let server_header = response
                .headers()
                .get(reqwest::header::SERVER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let body_text = match response.text() {
                Ok(text) => text,
                Err(_) => continue,
            };

            let mut runtime_info = match serde_json::from_str::<Value>(&body_text) {
                Ok(payload) => extract_helios_runtime_info(&payload),
                Err(_) => extract_runtime_info_from_plain_text(path, &body_text),
            };
            if runtime_info.runtime_product.is_none() {
                runtime_info.runtime_product = classify_runtime_product_from_response(
                    path,
                    &body_text,
                    server_header.as_deref(),
                );
            }
            if runtime_info.runtime_product.is_none() {
                runtime_info.runtime_product = Some("roboRIO".to_string());
            }
            if !runtime_info.has_data() {
                continue;
            }

            merge_runtime_info_with_richer_telemetry(&mut merged, runtime_info);
            if merged.firmware_version.is_some()
                && merged.os_version.is_some()
                && merged.telemetry_summary.is_some()
            {
                break 'probe;
            }
        }
    }

    if should_enrich_roborio_runtime_with_ssh(&merged) {
        if let Some(ssh_runtime_info) = fetch_roborio_runtime_info_over_ssh(ip) {
            merge_runtime_info_with_richer_telemetry(&mut merged, ssh_runtime_info);
        }
    }

    if merged.has_data() {
        Some(merged)
    } else {
        None
    }
}

fn should_enrich_roborio_runtime_with_ssh(runtime_info: &HeliosRuntimeInfo) -> bool {
    let metric_count = runtime_info
        .telemetry_summary
        .as_deref()
        .map(|summary| summary.split('·').count())
        .unwrap_or(0);
    metric_count < 3
}

fn fetch_roborio_runtime_info_over_ssh(ip: &str) -> Option<HeliosRuntimeInfo> {
    let output = run_roborio_ssh_command(
        ip,
        "sh -c 'uptime_s=$(cut -d. -f1 /proc/uptime 2>/dev/null | tr -cd \"0-9\"); cpu_pct=$(awk \"(/^cpu /){total=$2+$3+$4+$5+$6+$7+$8; busy=$2+$3+$4+$6+$7+$8; if (total>0) printf \\\"%.1f\\\", (busy/total)*100;}\" /proc/stat 2>/dev/null); mem_total_kib=$(awk \"/^MemTotal:/ {print \\$2}\" /proc/meminfo 2>/dev/null); mem_avail_kib=$(awk \"/^MemAvailable:/ {print \\$2}\" /proc/meminfo 2>/dev/null); mem_pct=$(awk -v total=\"$mem_total_kib\" -v avail=\"$mem_avail_kib\" \"BEGIN {if (total+0>0 && avail+0>=0) printf \\\"%.1f\\\", ((total-avail)/total)*100;}\"); voltage_raw=\"\"; for p in /sys/class/power_supply/*/voltage_now /sys/class/hwmon/hwmon*/in*_input; do [ -r \"$p\" ] || continue; voltage_raw=$(head -n1 \"$p\" 2>/dev/null | tr -cd \"0-9.\"); [ -n \"$voltage_raw\" ] && break; done; can_util=\"\"; for p in /tmp/frc-status.json /tmp/frc_status.json /var/volatile/tmp/frc-status.json /var/volatile/tmp/frc_status.json; do [ -r \"$p\" ] || continue; can_util=$(grep -Eom1 \"can(bus)?[^0-9]{0,12}[0-9]+(\\.[0-9]+)?\" \"$p\" 2>/dev/null | grep -Eom1 \"[0-9]+(\\.[0-9]+)?\"); [ -n \"$can_util\" ] && break; done; printf \"runtime_product=roboRIO\\n\"; [ -n \"$uptime_s\" ] && printf \"uptime_seconds=%s\\n\" \"$uptime_s\"; [ -n \"$cpu_pct\" ] && printf \"cpu_usage_pct=%s\\n\" \"$cpu_pct\"; [ -n \"$mem_pct\" ] && printf \"memory_usage_pct=%s\\n\" \"$mem_pct\"; if [ -n \"$voltage_raw\" ]; then voltage=$(awk -v raw=\"$voltage_raw\" \"BEGIN {v=raw+0; if (v>1000000) printf \\\"%.2f\\\", v/1000000; else if (v>1000) printf \\\"%.2f\\\", v/1000; else printf \\\"%.2f\\\", v;}\"); printf \"battery_voltage=%s\\n\" \"$voltage\"; fi; [ -n \"$can_util\" ] && printf \"canbus_utilization=%s\\n\" \"$can_util\";'",
        Duration::from_secs(6),
    )
    .ok()?;
    let payload = String::from_utf8(output.stdout).ok()?;
    if payload.trim().is_empty() {
        return None;
    }
    let mut runtime_info = extract_runtime_info_from_plain_text("ssh", &payload);
    if runtime_info.runtime_product.is_none() {
        runtime_info.runtime_product = Some("roboRIO".to_string());
    }
    if runtime_info.has_data() {
        Some(runtime_info)
    } else {
        None
    }
}

fn merge_runtime_info_with_richer_telemetry(
    target: &mut HeliosRuntimeInfo,
    candidate: HeliosRuntimeInfo,
) {
    if target.hostname.is_none() {
        target.hostname = candidate.hostname.clone();
    }
    if target.runtime_product.is_none() {
        target.runtime_product = candidate.runtime_product.clone();
    }
    if target.firmware_version.is_none() {
        target.firmware_version = candidate.firmware_version.clone();
    }
    if target.os_version.is_none() {
        target.os_version = candidate.os_version.clone();
    }
    target.telemetry_summary = select_richer_telemetry_summary(
        target.telemetry_summary.take(),
        candidate.telemetry_summary,
    );
}

fn maybe_enrich_with_photonvision_websocket(ip: &str, runtime_info: &mut HeliosRuntimeInfo) {
    let is_photonvision = runtime_info
        .runtime_product
        .as_deref()
        .map(is_photonvision_product)
        .unwrap_or(false);
    if !is_photonvision {
        return;
    }

    let has_core_details = runtime_info.firmware_version.is_some()
        && runtime_info.os_version.is_some()
        && runtime_info.telemetry_summary.is_some();
    if has_core_details {
        return;
    }

    if let Some(ws_runtime_info) = fetch_photonvision_websocket_runtime_info(ip) {
        if let Some(hostname) = ws_runtime_info.hostname {
            if runtime_info.hostname.is_none() {
                runtime_info.hostname = Some(hostname);
            }
        }
        if let Some(firmware_version) = ws_runtime_info.firmware_version {
            runtime_info.firmware_version = Some(firmware_version);
        }
        if let Some(os_version) = ws_runtime_info.os_version {
            if runtime_info.os_version.is_none() {
                runtime_info.os_version = Some(os_version);
            }
        }
        if let Some(telemetry_summary) = ws_runtime_info.telemetry_summary {
            // Prefer live websocket metrics over static HTTP probe values.
            runtime_info.telemetry_summary = Some(telemetry_summary);
        }
    }
}

fn fetch_photonvision_websocket_runtime_info(ip: &str) -> Option<HeliosRuntimeInfo> {
    let endpoint = format!("ws://{ip}:5800/websocket_data");
    let (mut socket, _) = tungstenite::connect(endpoint).ok()?;
    set_websocket_stream_timeouts(&mut socket, Duration::from_millis(350));

    let deadline = Instant::now() + Duration::from_millis(2600);
    let mut merged = HeliosRuntimeInfo {
        runtime_product: Some("PhotonVision".to_string()),
        ..HeliosRuntimeInfo::default()
    };

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
            merged.merge_missing(extract_helios_runtime_info(&payload));
            merge_photonvision_runtime_info_from_payload(&payload, &mut merged);
            if merged.is_complete() {
                break;
            }
        }

        if let Message::Ping(ping_data) = message {
            let _ = socket.send(Message::Pong(ping_data));
        }
    }

    let _ = socket.close(None);

    let has_runtime_details = merged.firmware_version.is_some()
        || merged.os_version.is_some()
        || merged.telemetry_summary.is_some();
    if has_runtime_details {
        Some(merged)
    } else {
        None
    }
}

fn merge_photonvision_runtime_info_from_payload(payload: &Value, out: &mut HeliosRuntimeInfo) {
    let mut fields = Vec::new();
    collect_scalar_fields(payload, String::new(), &mut fields);

    if out.hostname.is_none() {
        out.hostname = normalize_hostname_value(find_scalar_by_patterns(
            &fields,
            &[
                "settings.network.hostname",
                "network.hostname",
                "settings.general.hostname",
                "general.hostname",
                "hostname",
                "host_name",
                "device.hostname",
            ],
        ));
    }

    if let Some(version) = find_scalar_by_patterns(
        &fields,
        &[
            "settings.general.version",
            "general.version",
            "photon.version",
            "version",
        ],
    ) {
        out.firmware_version = Some(version);
    }

    if out.os_version.is_none() {
        out.os_version = find_scalar_by_patterns(
            &fields,
            &[
                "settings.general.hardware_platform",
                "general.hardware_platform",
                "hardware_platform",
                "settings.general.hardware_model",
                "hardware_model",
            ],
        );
    }

    if let Some(telemetry) = build_photonvision_telemetry_summary(&fields) {
        out.telemetry_summary = Some(telemetry);
    }
}

fn set_websocket_stream_timeouts(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
    }
}

fn parse_websocket_payload(message: &Message) -> Option<Value> {
    match message {
        Message::Binary(bytes) => {
            if let Ok(payload) = rmp_serde::from_slice::<Value>(bytes) {
                return Some(payload);
            }
            serde_json::from_slice::<Value>(bytes).ok()
        }
        Message::Text(text) => serde_json::from_str::<Value>(text).ok(),
        _ => None,
    }
}
