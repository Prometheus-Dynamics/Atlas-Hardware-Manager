use super::*;

#[path = "runtime_metrics_and_summary/helpers.rs"]
mod runtime_metrics_helpers;
pub(crate) use runtime_metrics_helpers::*;


pub(crate) fn build_helios_metrics_summary(payload: &Value) -> Option<String> {
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
