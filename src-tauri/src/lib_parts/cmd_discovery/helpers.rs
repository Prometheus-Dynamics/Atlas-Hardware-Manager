use super::*;

pub(crate) fn emit_discovery_progress(
    app: Option<&tauri::AppHandle>,
    devices: &[DiscoveredDevice],
    warnings: &[String],
    in_progress: bool,
) {
    let Some(app_handle) = app else {
        return;
    };

    let mut deduped_devices = devices.to_vec();
    dedupe_discovered_devices(&mut deduped_devices);
    let payload = DeviceDiscoveryProgressEvent {
        generated_at_epoch_ms: epoch_ms(),
        devices: deduped_devices,
        warnings: warnings.to_vec(),
        in_progress,
    };
    let _ = app_handle.emit(NETWORK_DISCOVERY_PROGRESS_EVENT, payload);
}

pub(crate) fn discovered_device_matches_identity(
    device: &DiscoveredDevice,
    ip: &str,
    mac: Option<&str>,
) -> bool {
    let target_ip = ip.trim();
    let same_ip = device
        .ip_address
        .as_deref()
        .map(str::trim)
        .map(|value| value == target_ip)
        .unwrap_or(false);
    if same_ip {
        return true;
    }

    let target_mac = mac.and_then(normalize_identity_mac);
    let device_mac = device
        .mac_address
        .as_deref()
        .and_then(normalize_identity_mac);
    matches!((target_mac, device_mac), (Some(left), Some(right)) if left == right)
}

pub(crate) fn normalize_identity_mac(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', ":");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) fn build_discovered_ip_device(
    candidate: IpDeviceCandidate,
    runtime_info: Option<HeliosRuntimeInfo>,
) -> DiscoveredDevice {
    let mut chips = Vec::new();
    if candidate.is_usb_link {
        chips.push("USB IP".to_string());
    } else {
        chips.push("Network IP".to_string());
    }

    let inferred_roborio = is_probable_roborio_ip(&candidate.ip)
        || candidate
            .mac
            .as_deref()
            .map(is_probable_roborio_mac)
            .unwrap_or(false);
    let runtime_product = runtime_info
        .as_ref()
        .and_then(|info| info.runtime_product.clone())
        .or_else(|| {
            if inferred_roborio {
                Some("roboRIO".to_string())
            } else {
                None
            }
        });
    let runtime_hostname = runtime_info
        .as_ref()
        .and_then(|info| normalize_hostname_value(info.hostname.clone()));
    if let Some(product) = runtime_product.as_deref() {
        chips.push(product.to_string());
    }
    if let Some(os_version) = runtime_info
        .as_ref()
        .and_then(|info| normalize_nonempty_text(info.os_version.clone()))
    {
        chips.push(format!("OS {}", compact_chip_value(&os_version)));
    }
    let display_name = runtime_hostname
        .clone()
        .unwrap_or_else(|| fallback_network_device_display_name(runtime_product.as_deref()));

    let state_text = candidate
        .state
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let usb_descriptor_summary = candidate.usb_identity.as_ref().map(|identity| {
        let manufacturer = identity
            .manufacturer
            .as_deref()
            .unwrap_or("Unknown Manufacturer");
        let product = identity.product.as_deref().unwrap_or("Unknown Product");
        format!(
            "USB {}:{} · {} {}",
            identity.vendor_id, identity.product_id, manufacturer, product
        )
    });
    let mut detail_segments = Vec::new();
    if let Some(hostname) = runtime_hostname {
        detail_segments.push(format!("Host {hostname}"));
    }
    if let Some(product) = runtime_product.as_ref() {
        detail_segments.push(product.clone());
    }
    if let Some(summary) = usb_descriptor_summary {
        detail_segments.push(summary);
    }
    if let Some(version) = runtime_info
        .as_ref()
        .and_then(|info| info.firmware_version.clone())
    {
        detail_segments.push(format!("FW {version}"));
    }
    if let Some(telemetry) = runtime_info
        .as_ref()
        .and_then(|info| info.telemetry_summary.clone())
    {
        detail_segments.push(telemetry);
    }
    detail_segments.push(format!("State: {state_text}"));
    let interface_name = candidate.interface.clone();
    let usb_location = if candidate.is_usb_link {
        if let Some(usb_path) = resolve_usb_topology_path_for_interface(&interface_name) {
            Some(format!("USB Path {usb_path} · Interface {interface_name}"))
        } else {
            Some(format!("Interface {interface_name}"))
        }
    } else {
        None
    };

    DiscoveredDevice {
        id: format!(
            "ip-{}",
            candidate
                .mac
                .clone()
                .unwrap_or_else(|| candidate.ip.replace('.', "-"))
        ),
        display_name,
        status: "online".to_string(),
        connection_chips: chips,
        ip_address: Some(candidate.ip),
        mac_address: candidate.mac,
        interface_name: Some(interface_name),
        usb_location,
        vendor_product: candidate
            .usb_identity
            .as_ref()
            .map(|identity| format!("{}:{}", identity.vendor_id, identity.product_id)),
        runtime_product,
        firmware_version: runtime_info
            .as_ref()
            .and_then(|info| info.firmware_version.clone()),
        os_version: runtime_info
            .as_ref()
            .and_then(|info| info.os_version.clone()),
        telemetry_summary: runtime_info
            .as_ref()
            .and_then(|info| info.telemetry_summary.clone()),
        detail: detail_segments.join(" · "),
    }
}

pub(crate) fn compact_chip_value(value: &str) -> String {
    let compacted = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if compacted.len() <= 28 {
        compacted
    } else {
        format!("{}...", compacted.chars().take(25).collect::<String>())
    }
}

pub(crate) fn fallback_network_device_display_name(runtime_product: Option<&str>) -> String {
    let Some(product) = runtime_product
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return "Unknown device".to_string();
    };
    format!("{product} device")
}

pub(crate) fn is_likely_roborio_ip_candidate(candidate: &IpDeviceCandidate) -> bool {
    if candidate
        .mac
        .as_deref()
        .map(is_probable_roborio_mac)
        .unwrap_or(false)
    {
        return true;
    }

    if !is_probable_roborio_ip(&candidate.ip) {
        return false;
    }

    matches!(
        candidate.state.as_deref().map(str::to_ascii_uppercase),
        Some(state) if matches!(state.as_str(), "REACHABLE" | "DELAY" | "PROBE" | "PERMANENT" | "STALE")
    )
}

pub(crate) fn has_network_candidate_signal(candidate: &IpDeviceCandidate) -> bool {
    if is_active_connection_neighbor_state(candidate.state.as_deref()) {
        return true;
    }
    candidate.is_usb_link
}
