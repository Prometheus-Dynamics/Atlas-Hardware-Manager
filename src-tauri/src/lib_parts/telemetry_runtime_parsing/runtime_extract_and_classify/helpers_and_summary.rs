use super::*;

pub(crate) fn collect_scalar_fields(value: &Value, prefix: String, out: &mut Vec<(String, String)>) {
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

pub(crate) fn collect_named_metric_fields(value: &Value, out: &mut Vec<(String, String)>) {
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

pub(crate) fn scalar_value_to_string(value: &Value) -> Option<String> {
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

pub(crate) fn normalize_hostname_value(value: Option<String>) -> Option<String> {
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

pub(crate) fn find_scalar_by_patterns(fields: &[(String, String)], patterns: &[&str]) -> Option<String> {
    find_scalar_entry_by_patterns(fields, patterns).map(|(_, value)| value)
}

pub(crate) fn find_scalar_entry_by_patterns(
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

pub(crate) fn build_telemetry_summary(fields: &[(String, String)]) -> Option<String> {
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

