use super::*;

pub(crate) fn nested_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

pub(crate) fn value_as_f64_lossy(value: &Value) -> Option<f64> {
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

pub(crate) fn value_as_u64_lossy(value: &Value) -> Option<u64> {
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

pub(crate) fn nested_f64(payload: &Value, path: &[&str]) -> Option<f64> {
    nested_value(payload, path).and_then(value_as_f64_lossy)
}

pub(crate) fn nested_u64(payload: &Value, path: &[&str]) -> Option<u64> {
    nested_value(payload, path).and_then(value_as_u64_lossy)
}

pub(crate) fn preferred_disk_usage_bytes(payload: &Value) -> (Option<u64>, Option<u64>) {
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

pub(crate) fn network_bitrates_bps(payload: &Value) -> (Option<f64>, Option<f64>) {
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

pub(crate) fn build_power_summary(payload: &Value) -> Option<String> {
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

pub(crate) fn format_unit_value(value: f64, unit: &str) -> String {
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

pub(crate) fn format_bytes_value(bytes: u64) -> String {
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

pub(crate) fn build_scalar_field_telemetry_summary(fields: &[(String, String)]) -> Option<String> {
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

pub(crate) fn compact_metric_label(key: &str) -> String {
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

pub(crate) fn build_photonvision_telemetry_summary(fields: &[(String, String)]) -> Option<String> {
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

pub(crate) fn normalize_match_key(input: &str) -> String {
    input
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

pub(crate) fn should_ignore_scalar_key(key: &str) -> bool {
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

pub(crate) fn score_key_pattern_match(key: &str, pattern: &str) -> Option<i32> {
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

pub(crate) fn value_quality_score(value: &str) -> i32 {
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

pub(crate) fn format_temperature_value(value: String) -> String {
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
