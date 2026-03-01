use super::*;

#[path = "runtime_extract_and_classify/helpers_and_summary.rs"]
mod runtime_extract_helpers_and_summary;
pub(crate) use runtime_extract_helpers_and_summary::*;


pub(crate) fn extract_helios_runtime_info(payload: &Value) -> HeliosRuntimeInfo {
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

pub(crate) fn extract_helios_runtime_info_from_payload(
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

pub(crate) fn merge_runtime_info(target: &mut HeliosRuntimeInfo, candidate: HeliosRuntimeInfo) {
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

pub(crate) fn runtime_info_richness_score(info: &HeliosRuntimeInfo) -> i32 {
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

pub(crate) fn telemetry_payload_views(payload: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    collect_telemetry_payload_views(payload, 0, &mut visited, &mut out);
    out
}

pub(crate) fn collect_telemetry_payload_views<'a>(
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

pub(crate) fn payload_contains_structured_helios_metrics(payload: &Value) -> bool {
    telemetry_payload_views(payload)
        .into_iter()
        .any(is_structured_helios_metrics_payload)
}

pub(crate) fn select_richer_telemetry_summary(
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

pub(crate) fn is_structured_helios_metrics_payload(payload: &Value) -> bool {
    nested_f64(payload, &["cpu", "usage_percent"]).is_some()
        || (nested_u64(payload, &["memory", "used_bytes"]).is_some()
            && nested_u64(payload, &["memory", "total_bytes"]).is_some())
        || payload.get("cpu_avg_pct").is_some()
        || payload.get("mem_used_bytes").is_some()
        || payload.get("mem_total_bytes").is_some()
        || payload.get("temps").is_some()
        || payload.get("disks").is_some()
}

pub(crate) fn classify_runtime_product_from_fields(fields: &[(String, String)]) -> Option<String> {
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

pub(crate) fn classify_runtime_product_from_response(
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

pub(crate) fn classify_runtime_product_from_text(text: &str, server_header: Option<&str>) -> Option<String> {
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

pub(crate) fn is_photonvision_product(product: &str) -> bool {
    product.to_ascii_lowercase().contains("photonvision")
}

pub(crate) fn is_roborio_product(product: &str) -> bool {
    let normalized = product.to_ascii_lowercase();
    normalized.contains("roborio") || normalized.contains("rio")
}
