fn extract_helios_runtime_info(payload: &Value) -> HeliosRuntimeInfo {
    let payload_views = telemetry_payload_views(payload);
    let strict_structured_summary = payload_contains_structured_helios_metrics(payload);
    let mut best_runtime_info: Option<HeliosRuntimeInfo> = None;
    let mut best_score = i32::MIN;

    for payload_view in payload_views {
        if strict_structured_summary && !is_structured_helios_metrics_payload(payload_view) {
            continue;
        }

        let runtime_info =
            extract_helios_runtime_info_from_payload(payload_view, strict_structured_summary);
        let score = runtime_info_richness_score(&runtime_info);
        if score > best_score {
            best_score = score;
            best_runtime_info = Some(runtime_info);
        } else if let Some(existing) = best_runtime_info.as_mut() {
            merge_runtime_info(existing, runtime_info);
        }
    }

    best_runtime_info
        .unwrap_or_else(|| extract_helios_runtime_info_from_payload(payload, false))
}

fn extract_helios_runtime_info_from_payload(
    payload: &Value,
    strict_structured_summary: bool,
) -> HeliosRuntimeInfo {
    let mut fields = Vec::new();
    collect_scalar_fields(payload, String::new(), &mut fields);
    collect_named_metric_fields(payload, &mut fields);

    let hostname = normalize_hostname_value(find_scalar_by_patterns(
        &fields,
        &[
            "hostname",
            "host_name",
            "device_hostname",
            "network_hostname",
            "settings.network.hostname",
            "settings.general.hostname",
            "general.hostname",
            "device.hostname",
        ],
    ));
    let firmware_version = find_scalar_by_patterns(
        &fields,
        &[
            "firmware_version",
            "fw_version",
            "software_version",
            "app_version",
            "general.version",
            "build_version",
            "firmware",
            "version",
        ],
    );
    let os_version = find_scalar_by_patterns(
        &fields,
        &[
            "os_version",
            "image_version",
            "image_tag",
            "release_version",
            "kernel_version",
            "kernel_release",
            "pretty_name",
            "build_version",
            "release",
            "hardware_platform",
        ],
    );
    let telemetry_summary = if strict_structured_summary {
        // Structured HeliOS telemetry is authoritative; avoid generic/scalar fallbacks.
        build_helios_metrics_summary(payload)
    } else {
        let helios_summary = build_helios_metrics_summary(payload);
        let generic_summary = build_telemetry_summary(&fields);
        let scalar_summary = build_scalar_field_telemetry_summary(&fields);
        if is_structured_helios_metrics_payload(payload) {
            helios_summary.or(generic_summary).or(scalar_summary)
        } else {
            select_richer_telemetry_summary(
                select_richer_telemetry_summary(helios_summary, generic_summary),
                scalar_summary,
            )
        }
    };
    let runtime_product = classify_runtime_product_from_fields(&fields);

    HeliosRuntimeInfo {
        hostname,
        runtime_product,
        firmware_version,
        os_version,
        telemetry_summary,
    }
}

fn merge_runtime_info(target: &mut HeliosRuntimeInfo, candidate: HeliosRuntimeInfo) {
    if target.hostname.is_none() {
        target.hostname = candidate.hostname;
    }
    if target.runtime_product.is_none() {
        target.runtime_product = candidate.runtime_product;
    }
    if target.firmware_version.is_none() {
        target.firmware_version = candidate.firmware_version;
    }
    if target.os_version.is_none() {
        target.os_version = candidate.os_version;
    }
    target.telemetry_summary = select_richer_telemetry_summary(
        target.telemetry_summary.take(),
        candidate.telemetry_summary,
    );
}

fn runtime_info_richness_score(info: &HeliosRuntimeInfo) -> i32 {
    let mut score = 0;
    if info.hostname.is_some() {
        score += 3;
    }
    if info.runtime_product.is_some() {
        score += 6;
    }
    if info.firmware_version.is_some() {
        score += 4;
    }
    if info.os_version.is_some() {
        score += 4;
    }
    if let Some(summary) = info.telemetry_summary.as_deref() {
        let segments = summary
            .split('·')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .count() as i32;
        score += 10 + segments;
    }
    score
}

fn telemetry_payload_views(payload: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    collect_telemetry_payload_views(payload, 0, &mut visited, &mut out);
    out
}

fn collect_telemetry_payload_views<'a>(
    payload: &'a Value,
    depth: usize,
    visited: &mut HashSet<usize>,
    out: &mut Vec<&'a Value>,
) {
    let pointer = payload as *const Value as usize;
    if !visited.insert(pointer) {
        return;
    }
    out.push(payload);

    if depth >= 3 {
        return;
    }

    let Value::Object(map) = payload else {
        return;
    };

    for key in [
        "data",
        "payload",
        "sample",
        "telemetry",
        "metrics",
        "resource",
        "resources",
        "message",
        "event",
    ] {
        if let Some(child) = map.get(key) {
            collect_telemetry_payload_views(child, depth + 1, visited, out);
        }
    }

    // Some telemetry feeds wrap samples under arbitrary object fields.
    for child in map.values() {
        if is_structured_helios_metrics_payload(child) {
            collect_telemetry_payload_views(child, depth + 1, visited, out);
        }
    }
}

fn payload_contains_structured_helios_metrics(payload: &Value) -> bool {
    telemetry_payload_views(payload)
        .into_iter()
        .any(is_structured_helios_metrics_payload)
}

fn select_richer_telemetry_summary(
    preferred: Option<String>,
    fallback: Option<String>,
) -> Option<String> {
    let preferred_count = preferred
        .as_deref()
        .map(|summary| summary.split('·').count())
        .unwrap_or(0);
    let fallback_count = fallback
        .as_deref()
        .map(|summary| summary.split('·').count())
        .unwrap_or(0);
    if fallback_count > preferred_count {
        fallback
    } else {
        preferred.or(fallback)
    }
}

fn is_structured_helios_metrics_payload(payload: &Value) -> bool {
    nested_f64(payload, &["cpu", "usage_percent"]).is_some()
        || (nested_u64(payload, &["memory", "used_bytes"]).is_some()
            && nested_u64(payload, &["memory", "total_bytes"]).is_some())
        || payload.get("cpu_avg_pct").is_some()
        || payload.get("mem_used_bytes").is_some()
        || payload.get("mem_total_bytes").is_some()
        || payload.get("temps").is_some()
        || payload.get("disks").is_some()
}

fn classify_runtime_product_from_fields(fields: &[(String, String)]) -> Option<String> {
    for (key, value) in fields {
        let key_lower = key.to_ascii_lowercase();
        let value_lower = value.to_ascii_lowercase();
        if key_lower.contains("roborio")
            || value_lower.contains("roborio")
            || value_lower.contains("ni linux real-time")
            || value_lower.contains("frc robo")
        {
            return Some("roboRIO".to_string());
        }
        if key_lower.contains("photon") || value_lower.contains("photonvision") {
            return Some("PhotonVision".to_string());
        }
        if key_lower.contains("limelight") || value_lower.contains("limelight") {
            return Some("Limelight".to_string());
        }
        if key_lower.contains("helios")
            || value_lower.contains("helios")
            || value_lower.contains("prometheus dynamics")
        {
            return Some("HeliOS".to_string());
        }
    }

    None
}

fn classify_runtime_product_from_response(
    path: &str,
    text: &str,
    server_header: Option<&str>,
) -> Option<String> {
    if path.starts_with("/v1/device/") || path == "/v1/health" {
        return Some("HeliOS".to_string());
    }

    // PhotonVision exposes a plain-text /api/status heartbeat: "not dead yet".
    if path == "/api/status" && text.trim().eq_ignore_ascii_case("not dead yet") {
        return Some("PhotonVision".to_string());
    }

    classify_runtime_product_from_text(text, server_header)
}

fn classify_runtime_product_from_text(text: &str, server_header: Option<&str>) -> Option<String> {
    let normalized = text.to_ascii_lowercase();
    if normalized.contains("roborio")
        || normalized.contains("ni linux real-time")
        || normalized.contains("frc robo")
    {
        return Some("roboRIO".to_string());
    }
    if normalized.contains("photonvision") || normalized.contains("photon vision") {
        return Some("PhotonVision".to_string());
    }
    if normalized.contains("limelight") {
        return Some("Limelight".to_string());
    }
    if normalized.contains("helios") {
        return Some("HeliOS".to_string());
    }

    if let Some(header) = server_header {
        let normalized_header = header.to_ascii_lowercase();
        if normalized_header.contains("photonvision") {
            return Some("PhotonVision".to_string());
        }
        if normalized_header.contains("limelight") {
            return Some("Limelight".to_string());
        }
    }

    None
}

fn is_photonvision_product(product: &str) -> bool {
    product.to_ascii_lowercase().contains("photonvision")
}

fn is_roborio_product(product: &str) -> bool {
    let normalized = product.to_ascii_lowercase();
    normalized.contains("roborio") || normalized.contains("rio")
}

fn collect_scalar_fields(value: &Value, prefix: String, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next_prefix = if prefix.is_empty() {
                    key.to_ascii_lowercase()
                } else {
                    format!("{prefix}.{}", key.to_ascii_lowercase())
                };
                collect_scalar_fields(child, next_prefix, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let next_prefix = format!("{prefix}[{index}]");
                collect_scalar_fields(child, next_prefix, out);
            }
        }
        _ => {
            if let Some(text) = scalar_value_to_string(value) {
                out.push((prefix, text));
            }
        }
    }
}

fn collect_named_metric_fields(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            let metric_name = map
                .get("name")
                .or_else(|| map.get("metric"))
                .or_else(|| map.get("key"))
                .or_else(|| map.get("label"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let metric_value = map
                .get("value")
                .or_else(|| map.get("reading"))
                .or_else(|| map.get("current"))
                .or_else(|| map.get("val"))
                .and_then(scalar_value_to_string);
            if let (Some(name), Some(value)) = (metric_name, metric_value) {
                out.push((name.to_ascii_lowercase(), value));
            }

            for child in map.values() {
                collect_named_metric_fields(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_named_metric_fields(child, out);
            }
        }
        _ => {}
    }
}

fn scalar_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn normalize_hostname_value(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 96 {
        return None;
    }
    if trimmed.contains('\n') || trimmed.contains('\r') || trimmed.contains("://") {
        return None;
    }

    let normalized = trimmed
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches('.')
        .trim();
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("localhost") {
        return None;
    }
    if normalized.chars().all(|character| character.is_ascii_digit() || character == '.') {
        return None;
    }
    if normalized
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        Some(normalized.to_string())
    } else {
        None
    }
}

fn find_scalar_by_patterns(fields: &[(String, String)], patterns: &[&str]) -> Option<String> {
    find_scalar_entry_by_patterns(fields, patterns).map(|(_, value)| value)
}

fn find_scalar_entry_by_patterns(
    fields: &[(String, String)],
    patterns: &[&str],
) -> Option<(String, String)> {
    let normalized_patterns = patterns
        .iter()
        .map(|pattern| normalize_match_key(pattern))
        .collect::<Vec<_>>();

    let mut best_match: Option<(i32, String, String)> = None;
    for (key, value) in fields {
        if value.trim().is_empty() {
            continue;
        }
        let normalized_key = normalize_match_key(key);
        if should_ignore_scalar_key(&normalized_key) {
            continue;
        }

        for pattern in &normalized_patterns {
            let Some(score) = score_key_pattern_match(&normalized_key, pattern) else {
                continue;
            };
            let current_score = score + value_quality_score(value);
            let should_replace = best_match
                .as_ref()
                .map(|(best_score, _, _)| current_score > *best_score)
                .unwrap_or(true);
            if should_replace {
                best_match = Some((current_score, key.clone(), value.clone()));
            }
        }
    }

    best_match.map(|(_, key, value)| (key, value))
}

fn build_telemetry_summary(fields: &[(String, String)]) -> Option<String> {
    let temperature = find_scalar_by_patterns(
        fields,
        &[
            "cpu_temp",
            "soc_temp",
            "temperature",
            "temp_c",
            "temp",
            "cpu_temperature",
            "soc_temperature",
            "board_temp",
            "cputemp",
        ],
    )
    .map(format_temperature_value);
    let uptime = find_scalar_entry_by_patterns(
        fields,
        &[
            "uptime_seconds",
            "uptime_sec",
            "uptime_s",
            "uptime_ms",
            "uptime_minutes",
            "uptime",
        ],
    )
    .map(|(key, value)| format_uptime_value_for_key(&key, &value));
    let cpu_load = find_scalar_by_patterns(
        fields,
        &[
            "cpu_util",
            "cpu_usage",
            "cpu_usage_pct",
            "cpu_utilization",
            "cpu_load",
            "cpu_avg_pct",
            "cpu_percent",
            "cpu_pct",
            "cpuutil",
            "cpuused",
            "cpubusy",
            "sys_cpu_usage",
            "syscpuusage",
            "system_cpu",
            "load_avg",
            "load",
        ],
    )
    .map(format_percentage_value);
    let ram_usage = find_scalar_by_patterns(
        fields,
        &[
            "ram_util",
            "ram_usage_pct",
            "memory_usage_pct",
            "memory_percent",
            "memory_pct",
            "mem_usage_pct",
            "memory_usage",
            "ramutil",
            "mem_util",
            "mem_percent",
            "memory_utilization",
            "memory_util",
            "memory_used_pct",
            "mem_used_pct",
            "memavailable",
        ],
    )
    .map(format_percentage_value)
    .or_else(|| {
        let used = find_scalar_by_patterns(
            fields,
            &[
                "mem_used_bytes",
                "memory_used_bytes",
                "ram_used_bytes",
                "memory_used",
                "ram_used",
                "memory_used_mib",
                "ram_used_mib",
                "memory_used_mb",
                "ram_used_mb",
                "memory_used_kb",
            ],
        )?;
        let total = find_scalar_by_patterns(
            fields,
            &[
                "mem_total_bytes",
                "memory_total_bytes",
                "ram_total_bytes",
                "memory_total",
                "ram_total",
                "memory_total_mib",
                "ram_total_mib",
                "memory_total_mb",
                "ram_total_mb",
                "memory_total_kb",
            ],
        );
        total
            .as_deref()
            .and_then(|total_value| format_usage_ratio_percentage(&used, total_value))
    });
    let disk_usage = find_scalar_by_patterns(
        fields,
        &[
            "disk_util_pct",
            "disk_usage_pct",
            "disk_usage",
            "diskutilpct",
            "diskutil",
            "storage_usage_pct",
            "rootfs_used_pct",
            "root_usage_pct",
        ],
    )
    .map(format_percentage_value);
    let voltage = find_scalar_by_patterns(fields, &["voltage", "bus_voltage", "input_voltage"])
        .map(format_voltage_value);
    let voltage = voltage.or_else(|| {
        find_scalar_by_patterns(
            fields,
            &[
                "battery_voltage",
                "batt_voltage",
                "vin_voltage",
                "inputvoltage",
                "robot_voltage",
                "robotvoltage",
                "v_in",
            ],
        )
        .map(format_voltage_value)
    });
    let canbus_usage = find_scalar_by_patterns(
        fields,
        &[
            "can_util",
            "can_usage",
            "can_usage_pct",
            "can_percent",
            "canbus_util",
            "canbus_usage",
            "canbus_utilization",
            "can_bus_util",
            "can_bus_usage",
            "can_utilization",
            "canstatusutilization",
            "canstatusbusutilization",
        ],
    )
    .map(format_percentage_value);
    let tx_bitrate =
        find_scalar_by_patterns(
            fields,
            &[
                "sent_bit_rate",
                "sentbitrate",
                "tx_bitrate",
                "tx_bps",
                "network_tx_bps",
                "tx_rate_bps",
            ],
        )
            .map(format_bitrate_value);
    let rx_bitrate =
        find_scalar_by_patterns(
            fields,
            &[
                "recv_bit_rate",
                "recvbitrate",
                "rx_bitrate",
                "rx_bps",
                "network_rx_bps",
                "rx_rate_bps",
            ],
        )
            .map(format_bitrate_value);

    let mut parts = Vec::new();
    if let Some(value) = temperature {
        parts.push(format!("Temp {value}"));
    }
    if let Some(value) = uptime {
        parts.push(format!("Uptime {value}"));
    }
    if let Some(value) = cpu_load {
        parts.push(format!("CPU {value}"));
    }
    if let Some(value) = ram_usage {
        parts.push(format!("RAM {value}"));
    }
    if let Some(value) = disk_usage {
        parts.push(format!("Disk {value}"));
    }
    if let Some(value) = voltage {
        parts.push(format!("Voltage {value}"));
    }
    if let Some(value) = canbus_usage {
        parts.push(format!("CAN {value}"));
    }
    if let Some(value) = tx_bitrate {
        parts.push(format!("TX {value}"));
    }
    if let Some(value) = rx_bitrate {
        parts.push(format!("RX {value}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn build_helios_metrics_summary(payload: &Value) -> Option<String> {
    let mut fields = Vec::new();
    collect_scalar_fields(payload, String::new(), &mut fields);
    collect_named_metric_fields(payload, &mut fields);

    let cpu_load = nested_f64(payload, &["cpu", "usage_percent"])
        .or_else(|| payload.get("cpu_avg_pct").and_then(value_as_f64_lossy))
        .map(|value| format_percentage_value(value.to_string()))
        .or_else(|| {
            find_scalar_by_patterns(
                &fields,
                &[
                    "cpu_avg_pct",
                    "cpu_usage",
                    "cpu_util",
                    "cpu_load",
                    "cpu_percent",
                    "cpu_pct",
                ],
            )
            .map(format_percentage_value)
        });

    let cpu_temp = nested_f64(payload, &["cpu", "temperature_c"])
        .map(|value| format_temperature_value(value.to_string()))
        .or_else(|| {
            payload
                .get("temps")
                .and_then(Value::as_array)
                .and_then(|temps| {
                    temps
                        .iter()
                        .filter_map(|entry| entry.get("temperature_c").and_then(value_as_f64_lossy))
                        .max_by(|left, right| {
                            left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
                        })
                })
                .map(|value| format_temperature_value(value.to_string()))
        })
        .or_else(|| {
            find_scalar_by_patterns(
                &fields,
                &[
                    "cpu_temp",
                    "soc_temp",
                    "temperature",
                    "temp_c",
                    "temp",
                    "cpu_temperature",
                    "soc_temperature",
                    "board_temp",
                    "cputemp",
                ],
            )
            .map(format_temperature_value)
        });

    let memory_used_bytes = nested_u64(payload, &["memory", "used_bytes"])
        .or_else(|| payload.get("mem_used_bytes").and_then(value_as_u64_lossy));
    let memory_total_bytes = nested_u64(payload, &["memory", "total_bytes"])
        .or_else(|| payload.get("mem_total_bytes").and_then(value_as_u64_lossy));
    let ram_usage = memory_used_bytes
        .zip(memory_total_bytes)
        .and_then(|(used, total)| format_usage_ratio_percentage(&used.to_string(), &total.to_string()))
        .or_else(|| {
        find_scalar_by_patterns(
            &fields,
            &[
                "ram_util",
                "ram_usage_pct",
                "memory_usage_pct",
                "memory_percent",
                "memory_pct",
                "mem_usage_pct",
                "memory_usage",
                "mem_percent",
            ],
        )
        .map(format_percentage_value)
    });

    let (disk_used_bytes, disk_total_bytes) = preferred_disk_usage_bytes(payload);
    let disk_usage = disk_used_bytes
        .zip(disk_total_bytes)
        .and_then(|(used, total)| format_usage_ratio_percentage(&used.to_string(), &total.to_string()))
        .or_else(|| {
            find_scalar_by_patterns(
                &fields,
                &[
                    "disk_util_pct",
                    "disk_usage_pct",
                    "disk_usage",
                    "diskutilpct",
                    "diskutil",
                    "storage_usage_pct",
                    "rootfs_used_pct",
                    "root_usage_pct",
                ],
            )
            .map(format_percentage_value)
        });
    let disk_detail = disk_used_bytes
        .zip(disk_total_bytes)
        .map(|(used, total)| format!("{}/{}", format_bytes_value(used), format_bytes_value(total)));

    let gpu_usage = nested_f64(payload, &["gpu", "usage_percent"])
        .map(|value| format_percentage_value(value.to_string()));
    let gpu_temp = nested_f64(payload, &["gpu", "temperature_c"])
        .map(|value| format_temperature_value(value.to_string()));
    let gpu_memory_usage = nested_u64(payload, &["gpu", "memory", "used_bytes"])
        .zip(nested_u64(payload, &["gpu", "memory", "total_bytes"]))
        .and_then(|(used, total)| format_usage_ratio_percentage(&used.to_string(), &total.to_string()));

    let power_summary = build_power_summary(payload);
    let (rx_bps, tx_bps) = network_bitrates_bps(payload);
    let rx_bitrate = rx_bps.map(|value| format_bitrate_value(value.to_string()));
    let tx_bitrate = tx_bps.map(|value| format_bitrate_value(value.to_string()));

    let uptime = find_scalar_entry_by_patterns(
        &fields,
        &[
            "uptime_seconds",
            "uptime_sec",
            "uptime_s",
            "uptime_ms",
            "uptime_minutes",
            "uptime",
        ],
    )
    .map(|(key, value)| format_uptime_value_for_key(&key, &value));

    let mut parts = Vec::new();
    if let Some(value) = cpu_load {
        parts.push(format!("CPU {value}"));
    }
    if let Some(value) = cpu_temp {
        parts.push(format!("Temp {value}"));
    }
    if let Some(value) = ram_usage {
        parts.push(format!("RAM {value}"));
    }
    if let Some(value) = gpu_usage {
        parts.push(format!("GPU {value}"));
    }
    if let Some(value) = gpu_temp {
        parts.push(format!("GpuTemp {value}"));
    }
    if let Some(value) = gpu_memory_usage {
        parts.push(format!("VRAM {value}"));
    }
    if let Some(value) = disk_usage {
        parts.push(format!("Disk {value}"));
    }
    if let Some(value) = disk_detail {
        parts.push(format!("DiskBytes {value}"));
    }
    if let Some(value) = power_summary {
        parts.push(format!("Power {value}"));
    }
    if let Some(value) = rx_bitrate {
        parts.push(format!("RX {value}"));
    }
    if let Some(value) = tx_bitrate {
        parts.push(format!("TX {value}"));
    }
    if let Some(value) = uptime {
        parts.push(format!("Uptime {value}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn nested_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn value_as_f64_lossy(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        return Some(number);
    }
    if let Some(number) = value.as_u64() {
        return Some(number as f64);
    }
    if let Some(number) = value.as_i64() {
        return Some(number as f64);
    }
    value.as_str().and_then(parse_numeric_value)
}

fn value_as_u64_lossy(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    if let Some(number) = value.as_i64() {
        if number >= 0 {
            return Some(number as u64);
        }
    }
    if let Some(number) = value.as_f64() {
        if number.is_finite() && number >= 0.0 {
            return Some(number.round() as u64);
        }
    }
    value
        .as_str()
        .and_then(parse_numeric_value)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number.round() as u64)
}

fn nested_f64(payload: &Value, path: &[&str]) -> Option<f64> {
    nested_value(payload, path).and_then(value_as_f64_lossy)
}

fn nested_u64(payload: &Value, path: &[&str]) -> Option<u64> {
    nested_value(payload, path).and_then(value_as_u64_lossy)
}

fn preferred_disk_usage_bytes(payload: &Value) -> (Option<u64>, Option<u64>) {
    if let Some(total) = nested_u64(payload, &["disk", "total_bytes"]) {
        let used = nested_u64(payload, &["disk", "used_bytes"]).or_else(|| {
            nested_u64(payload, &["disk", "free_bytes"]).map(|free| total.saturating_sub(free))
        });
        return (used, Some(total));
    }

    let Some(disks) = payload.get("disks").and_then(Value::as_array) else {
        return (None, None);
    };
    let Some(preferred) = disks
        .iter()
        .find(|disk| disk.get("mount").and_then(Value::as_str) == Some("/"))
        .or_else(|| disks.first())
    else {
        return (None, None);
    };
    let total = preferred.get("total_bytes").and_then(value_as_u64_lossy);
    let used = preferred
        .get("used_bytes")
        .and_then(value_as_u64_lossy)
        .or_else(|| {
            total.zip(
                preferred
                    .get("free_bytes")
                    .and_then(value_as_u64_lossy)
                    .or_else(|| preferred.get("available_bytes").and_then(value_as_u64_lossy)),
            )
            .map(|(total_value, free_value)| total_value.saturating_sub(free_value))
        });
    (used, total)
}

fn network_bitrates_bps(payload: &Value) -> (Option<f64>, Option<f64>) {
    let direct_rx = nested_f64(payload, &["network", "rx_bytes_per_sec"]).map(|value| value * 8.0);
    let direct_tx = nested_f64(payload, &["network", "tx_bytes_per_sec"]).map(|value| value * 8.0);
    if direct_rx.is_some() || direct_tx.is_some() {
        return (direct_rx, direct_tx);
    }

    let Some(interfaces) = nested_value(payload, &["network", "interfaces"]).and_then(Value::as_array)
    else {
        return (None, None);
    };
    let mut rx_sum_bps = 0.0;
    let mut tx_sum_bps = 0.0;
    let mut saw_rx = false;
    let mut saw_tx = false;
    for interface in interfaces {
        if let Some(rx) = interface
            .get("rx_bytes_per_sec")
            .and_then(value_as_f64_lossy)
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            rx_sum_bps += rx * 8.0;
            saw_rx = true;
        }
        if let Some(tx) = interface
            .get("tx_bytes_per_sec")
            .and_then(value_as_f64_lossy)
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            tx_sum_bps += tx * 8.0;
            saw_tx = true;
        }
    }

    let rx = if saw_rx { Some(rx_sum_bps) } else { None };
    let tx = if saw_tx { Some(tx_sum_bps) } else { None };
    (rx, tx)
}

fn build_power_summary(payload: &Value) -> Option<String> {
    let watts = nested_f64(payload, &["power", "watts"]);
    let volts = nested_f64(payload, &["power", "volts"]);
    let amps = nested_f64(payload, &["power", "amps"]);

    let mut segments = Vec::new();
    if let Some(value) = watts {
        segments.push(format_unit_value(value, "W"));
    }
    if let Some(value) = volts {
        segments.push(format_unit_value(value, "V"));
    }
    if let Some(value) = amps {
        segments.push(format_unit_value(value, "A"));
    }

    if segments.is_empty() {
        None
    } else {
        Some(segments.join(" "))
    }
}

fn format_unit_value(value: f64, unit: &str) -> String {
    if !value.is_finite() {
        return format!("0{unit}");
    }
    let magnitude = value.abs();
    if magnitude >= 100.0 {
        format!("{value:.0}{unit}")
    } else if magnitude >= 10.0 {
        format!("{value:.1}{unit}")
    } else {
        format!("{value:.2}{unit}")
    }
}

fn format_bytes_value(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;

    let value = bytes as f64;
    if value >= TIB {
        format!("{:.1} TiB", value / TIB)
    } else if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn build_scalar_field_telemetry_summary(fields: &[(String, String)]) -> Option<String> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    for (key, value) in fields {
        let trimmed_value = value.trim();
        if trimmed_value.is_empty() {
            continue;
        }
        let normalized_key = normalize_match_key(key);
        if normalized_key.is_empty() || should_ignore_scalar_key(&normalized_key) {
            continue;
        }
        let memory_raw_count = (normalized_key.contains("mem")
            || normalized_key.contains("memory")
            || normalized_key.contains("ram"))
            && (normalized_key.contains("bytes")
                || normalized_key.ends_with("memused")
                || normalized_key.ends_with("memtotal")
                || normalized_key.ends_with("memoryused")
                || normalized_key.ends_with("memorytotal")
                || normalized_key.ends_with("ramused")
                || normalized_key.ends_with("ramtotal"));
        if memory_raw_count {
            continue;
        }
        if !seen.insert(normalized_key.clone()) {
            continue;
        }
        if matches!(
            normalized_key.as_str(),
            "name" | "metric" | "key" | "label" | "value" | "reading" | "current"
        ) {
            continue;
        }

        let label = compact_metric_label(key);
        let formatted = if normalized_key.contains("uptime") {
            format_uptime_value_for_key(key, trimmed_value)
        } else if normalized_key.contains("temp") {
            format_temperature_value(trimmed_value.to_string())
        } else if normalized_key.contains("voltage") || normalized_key.contains("vin") {
            format_voltage_value(trimmed_value.to_string())
        } else if normalized_key.contains("bitrate")
            || normalized_key.contains("bps")
            || normalized_key.contains("tx")
            || normalized_key.contains("rx")
        {
            format_bitrate_value(trimmed_value.to_string())
        } else if normalized_key.contains("pct")
            || normalized_key.contains("percent")
            || normalized_key.contains("usage")
            || normalized_key.contains("util")
            || normalized_key.contains("load")
        {
            format_percentage_value(trimmed_value.to_string())
        } else {
            trimmed_value.to_string()
        };

        parts.push(format!("{label} {formatted}"));
        if parts.len() >= 12 {
            break;
        }
    }

    if parts.len() < 2 {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn compact_metric_label(key: &str) -> String {
    let tail = key
        .rsplit('.')
        .next()
        .unwrap_or(key)
        .split('[')
        .next()
        .unwrap_or(key)
        .trim()
        .to_ascii_lowercase();
    let compact = tail
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    if compact.is_empty() {
        "metric".to_string()
    } else {
        compact
    }
}

fn build_photonvision_telemetry_summary(fields: &[(String, String)]) -> Option<String> {
    let temperature = find_scalar_by_patterns(
        fields,
        &["metrics.cpu_temp", "metrics.cputemp", "cpu_temp", "cputemp"],
    )
    .map(format_temperature_value);
    let cpu_load = find_scalar_by_patterns(
        fields,
        &[
            "metrics.cpu_util",
            "metrics.cpuutil",
            "metrics.cpu_usage",
            "cpu_util",
            "cpuutil",
            "cpu_usage",
        ],
    )
    .map(format_percentage_value);
    let ram_used = find_scalar_by_patterns(
        fields,
        &[
            "metrics.ram_util",
            "metrics.ramutil",
            "metrics.mem_used",
            "metrics.memory_used",
            "ram_util",
            "ramutil",
            "mem_used",
            "memory_used",
        ],
    );
    let ram_total = find_scalar_by_patterns(
        fields,
        &[
            "metrics.ram_mem",
            "metrics.rammem",
            "metrics.mem_total",
            "metrics.memory_total",
            "ram_mem",
            "rammem",
            "mem_total",
            "memory_total",
        ],
    );
    let ram_usage = match (ram_used.as_deref(), ram_total.as_deref()) {
        (Some(used), Some(total)) => format_usage_ratio_percentage(used, total),
        _ => None,
    };
    let disk_usage = find_scalar_by_patterns(
        fields,
        &[
            "metrics.disk_util_pct",
            "metrics.diskutilpct",
            "disk_util_pct",
            "diskutilpct",
        ],
    )
    .map(format_percentage_value);
    let uptime = find_scalar_entry_by_patterns(
        fields,
        &[
            "metrics.uptime",
            "metrics.uptime_ms",
            "metrics.uptime_minutes",
            "uptime_seconds",
            "uptime_sec",
            "uptime_s",
            "uptime",
        ],
    )
    .map(|(key, value)| format_uptime_value_for_key(&key, &value));
    let tx_bitrate = find_scalar_by_patterns(
        fields,
        &[
            "metrics.sent_bit_rate",
            "metrics.sentbitrate",
            "sent_bit_rate",
            "sentbitrate",
        ],
    )
    .map(format_bitrate_value);
    let rx_bitrate = find_scalar_by_patterns(
        fields,
        &[
            "metrics.recv_bit_rate",
            "metrics.recvbitrate",
            "recv_bit_rate",
            "recvbitrate",
        ],
    )
    .map(format_bitrate_value);

    let mut parts = Vec::new();
    if let Some(value) = temperature {
        parts.push(format!("Temp {value}"));
    }
    if let Some(value) = cpu_load {
        parts.push(format!("CPU {value}"));
    }
    if let Some(value) = ram_usage {
        parts.push(format!("RAM {value}"));
    }
    if let Some(value) = disk_usage {
        parts.push(format!("Disk {value}"));
    }
    if let Some(value) = uptime {
        parts.push(format!("Uptime {value}"));
    }
    if let Some(value) = tx_bitrate {
        parts.push(format!("TX {value}"));
    }
    if let Some(value) = rx_bitrate {
        parts.push(format!("RX {value}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn normalize_match_key(input: &str) -> String {
    input
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn should_ignore_scalar_key(key: &str) -> bool {
    key.contains("apiversion")
        || key.contains("schema")
        || key.contains("contenttype")
        || key.contains("contentlength")
        || key.ends_with("label")
        || key.ends_with("labels")
        || key.ends_with("unit")
        || key.ends_with("units")
        || key.ends_with("symbol")
        || key.ends_with("title")
        || key.ends_with("description")
        || key.ends_with("desc")
}

fn score_key_pattern_match(key: &str, pattern: &str) -> Option<i32> {
    if key == pattern {
        return Some(120);
    }
    if key.ends_with(pattern) {
        return Some(100);
    }
    if key.starts_with(pattern) {
        return Some(85);
    }
    if key.contains(pattern) {
        return Some(70);
    }
    None
}

fn value_quality_score(value: &str) -> i32 {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return -100;
    }
    if matches!(trimmed.to_ascii_lowercase().as_str(), "n/a" | "na" | "none" | "null") {
        return -80;
    }
    if trimmed.len() > 120 {
        return -20;
    }
    let has_numeric = parse_numeric_value(trimmed).is_some();
    if has_numeric {
        return 20;
    }
    if trimmed
        .chars()
        .all(|character| character.is_ascii_uppercase() || character == '_' || character == ' ')
    {
        return -20;
    }
    if trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || ".-_ ".contains(character))
    {
        8
    } else {
        0
    }
}

fn format_temperature_value(value: String) -> String {
    if value.contains('C') || value.contains('c') {
        return value;
    }

    let normalized = value.trim();
    if normalized.is_empty() {
        return value;
    }

    if let Ok(mut numeric) = normalized.parse::<f64>() {
        if numeric > 300.0 {
            numeric /= 1000.0;
        }
        return format!("{numeric:.1}C");
    }

    format!("{normalized}C")
}
