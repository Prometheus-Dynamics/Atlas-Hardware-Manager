use super::*;

pub(crate) fn format_voltage_value(value: String) -> String {
    if value.contains('V') || value.contains('v') {
        return value;
    }

    let normalized = value.trim();
    if normalized.is_empty() {
        return value;
    }

    if let Ok(mut numeric) = normalized.parse::<f64>() {
        if numeric > 60.0 {
            numeric /= 1000.0;
        }
        return format!("{numeric:.2}V");
    }

    format!("{normalized}V")
}

pub(crate) fn format_uptime_value_for_key(key: &str, value: &str) -> String {
    let trimmed = value.trim();
    let numeric = match parse_numeric_value(trimmed) {
        Some(value) if value.is_finite() && value >= 0.0 => value,
        _ => return value.to_string(),
    };
    let normalized_key = normalize_match_key(key);

    let seconds = if normalized_key.contains("uptimems")
        || normalized_key.contains("millisecond")
        || normalized_key.ends_with("ms")
    {
        (numeric / 1000.0).round() as u64
    } else if normalized_key.contains("uptimemin")
        || normalized_key.contains("minutes")
        || normalized_key.ends_with("min")
    {
        (numeric * 60.0).round() as u64
    } else if normalized_key.contains("uptimehour") || normalized_key.ends_with("hours") {
        (numeric * 3600.0).round() as u64
    } else if normalized_key.contains("uptimeday") || normalized_key.ends_with("days") {
        (numeric * 86_400.0).round() as u64
    } else if numeric >= 1_000_000_000_000.0 {
        let epoch_seconds = (numeric / 1000.0).round() as u64;
        let now_seconds = epoch_ms() / 1000;
        now_seconds.saturating_sub(epoch_seconds)
    } else if numeric >= 1_000_000_000.0 {
        let epoch_seconds = numeric.round() as u64;
        let now_seconds = epoch_ms() / 1000;
        now_seconds.saturating_sub(epoch_seconds)
    } else if (86_400.0..=10_000_000.0).contains(&numeric) && (numeric as u64).is_multiple_of(1000) {
        (numeric / 1000.0).round() as u64
    } else {
        numeric.round() as u64
    };

    format_duration_human(seconds)
}

pub(crate) fn format_duration_human(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    }
}

pub(crate) fn format_percentage_value(value: String) -> String {
    if value.contains('%') {
        return value;
    }

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return value;
    }

    let Ok(mut numeric) = trimmed.parse::<f64>() else {
        return value;
    };
    if numeric <= 1.0 {
        numeric *= 100.0;
    }

    if numeric.abs() < 10.0 {
        format!("{numeric:.1}%")
    } else {
        format!("{numeric:.0}%")
    }
}

pub(crate) fn format_bitrate_value(value: String) -> String {
    if value.to_ascii_lowercase().contains("bps") {
        return value;
    }

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return value;
    }

    let Ok(numeric) = trimmed.parse::<f64>() else {
        return value;
    };
    if numeric >= 1_000_000_000.0 {
        format!("{:.2} Gbps", numeric / 1_000_000_000.0)
    } else if numeric >= 1_000_000.0 {
        format!("{:.2} Mbps", numeric / 1_000_000.0)
    } else if numeric >= 1_000.0 {
        format!("{:.1} Kbps", numeric / 1_000.0)
    } else {
        format!("{numeric:.0} bps")
    }
}

pub(crate) fn parse_numeric_value(value: &str) -> Option<f64> {
    let mut token = String::new();
    let mut seen_digit = false;
    let mut seen_decimal = false;
    let mut seen_sign = false;

    for character in value.chars() {
        if character.is_ascii_whitespace() && token.is_empty() {
            continue;
        }
        if character == '+' || character == '-' {
            if seen_digit || seen_decimal || seen_sign {
                break;
            }
            seen_sign = true;
            token.push(character);
            continue;
        }
        if character.is_ascii_digit() {
            seen_digit = true;
            token.push(character);
            continue;
        }
        if character == '.' {
            if seen_decimal {
                break;
            }
            seen_decimal = true;
            token.push(character);
            continue;
        }
        break;
    }

    if !seen_digit {
        return None;
    }

    token.parse::<f64>().ok()
}

pub(crate) fn format_usage_ratio_percentage(used: &str, total: &str) -> Option<String> {
    let used_value = parse_numeric_value(used)?;
    let total_value = parse_numeric_value(total)?;
    if total_value <= 0.0 {
        return None;
    }

    let percent = (used_value / total_value) * 100.0;
    if !percent.is_finite() {
        return None;
    }

    let bounded = percent.clamp(0.0, 999.0);
    if bounded < 10.0 {
        Some(format!("{bounded:.1}%"))
    } else {
        Some(format!("{bounded:.0}%"))
    }
}

pub(crate) fn select_likely_mounted_target(targets: &[FlashTarget]) -> Option<&FlashTarget> {
    let mut candidates = targets
        .iter()
        .filter(|target| is_likely_helios_mounted_target(target))
        .collect::<Vec<_>>();

    if candidates.len() == 1 {
        return candidates.pop();
    }

    None
}

pub(crate) fn is_likely_helios_mounted_target(target: &FlashTarget) -> bool {
    let model = target.model.to_ascii_lowercase();
    if model.trim().is_empty() {
        return false;
    }

    [
        "raspberry",
        "helios",
        "rpi",
        "gadget",
        "file-stor",
        "rpiboot",
    ]
    .iter()
    .any(|needle| model.contains(needle))
}

pub(crate) const NEIGHBOR_PROBE_SETTLE_DELAY_MS: u64 = 70;
pub(crate) const ROBOT_TCP_PROBE_TIMEOUT_MS: u64 = 180;
const MAX_ROBORIO_ENDPOINT_PROBE_CANDIDATES: usize = 6;
pub(crate) const ROBORIO_HOSTNAME_RESOLVE_TIMEOUT_MS: u64 = 220;

pub(crate) fn discover_network_neighbors() -> Result<Vec<NetworkNeighbor>, String> {
    Ok(build_network_neighbors_from_raw_entries(
        discover_neighbor_entries()?,
    ))
}

pub(crate) fn discover_network_neighbors_fast() -> Result<Vec<NetworkNeighbor>, String> {
    let entries = dedupe_raw_neighbor_entries(collect_neighbor_entries());
    Ok(build_network_neighbors_from_raw_entries(entries))
}

pub(crate) fn build_network_neighbors_from_raw_entries(entries: Vec<RawNeighborEntry>) -> Vec<NetworkNeighbor> {
    let mut neighbors = Vec::new();
    for entry in entries {
        if matches!(
            entry.state.as_deref(),
            Some("FAILED") | Some("INCOMPLETE") | Some("NONE")
        ) {
            continue;
        }

        neighbors.push(NetworkNeighbor {
            ip: entry.ip,
            mac: entry.mac.clone(),
            interface: entry.interface,
            state: entry.state,
            is_helios_candidate: entry
                .mac
                .as_deref()
                .map(is_raspberry_pi_mac)
                .unwrap_or(false),
        });
    }

    neighbors.sort_by(|left, right| {
        right
            .is_helios_candidate
            .cmp(&left.is_helios_candidate)
            .then_with(|| left.ip.cmp(&right.ip))
    });

    neighbors
}

pub(crate) fn is_raspberry_pi_mac(mac: &str) -> bool {
    let normalized = mac.to_lowercase();
    let prefixes = ["b8:27:eb", "dc:a6:32", "e4:5f:01", "d8:3a:dd", "2c:cf:67"];
    prefixes.iter().any(|prefix| normalized.starts_with(prefix))
}

pub(crate) fn find_roborio_neighbor(neighbors: &[NetworkNeighbor]) -> Option<NetworkNeighbor> {
    let mut candidates = neighbors
        .iter()
        .filter(|neighbor| {
            is_probable_roborio_ip(&neighbor.ip)
                || neighbor
                    .mac
                    .as_deref()
                    .map(is_probable_roborio_mac)
                    .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();

    let has_live_strong_candidate = candidates
        .iter()
        .any(|neighbor| {
            has_strong_roborio_signal_from_neighbor(neighbor)
                && is_live_roborio_neighbor_candidate(neighbor)
        });

    let mut mdns_ips = Vec::new();
    if !has_live_strong_candidate {
        mdns_ips = discover_roborio_mdns_ips();
        for ip in &mdns_ips {
            let interface = resolve_interface_for_ip(ip);
            if interface
                .as_deref()
                .map(should_ignore_interface)
                .unwrap_or(false)
            {
                continue;
            }

            let candidate = NetworkNeighbor {
                ip: ip.clone(),
                mac: None,
                interface,
                state: Some("REACHABLE".to_string()),
                is_helios_candidate: false,
            };
            merge_roborio_candidate(&mut candidates, candidate);
        }

        let has_live_mdns_candidate = candidates.iter().any(|neighbor| {
            mdns_ips.iter().any(|ip| ip == &neighbor.ip) && is_live_roborio_neighbor_candidate(neighbor)
        });
        if !has_live_mdns_candidate {
            for ip in discover_roborio_driver_station_or_local_ips() {
                let interface = resolve_interface_for_ip(&ip);
                if interface
                    .as_deref()
                    .map(should_ignore_interface)
                    .unwrap_or(false)
                {
                    continue;
                }

                let candidate = NetworkNeighbor {
                    ip,
                    mac: None,
                    interface,
                    state: Some("REACHABLE".to_string()),
                    is_helios_candidate: false,
                };
                merge_roborio_candidate(&mut candidates, candidate);
            }
        }
    }

    if let Some(strong_candidate) = candidates
        .iter()
        .filter(|neighbor| {
            has_strong_roborio_signal_from_neighbor(neighbor)
                && is_live_roborio_neighbor_candidate(neighbor)
        })
        .max_by_key(|neighbor| roborio_neighbor_score(neighbor, &mdns_ips))
        .cloned()
    {
        return Some(strong_candidate);
    }

    if let Some(mdns_candidate) = candidates
        .iter()
        .filter(|neighbor| {
            mdns_ips.iter().any(|ip| ip == &neighbor.ip) && is_live_roborio_neighbor_candidate(neighbor)
        })
        .max_by_key(|neighbor| roborio_neighbor_score(neighbor, &mdns_ips))
        .cloned()
    {
        return Some(mdns_candidate);
    }

    candidates.sort_by_key(|neighbor| Reverse(roborio_neighbor_score(neighbor, &mdns_ips)));
    for candidate in candidates
        .iter()
        .take(MAX_ROBORIO_ENDPOINT_PROBE_CANDIDATES)
    {
        if is_roborio_candidate_reachable(&candidate.ip) {
            return Some(candidate.clone());
        }
    }

    None
}

pub(crate) fn is_live_roborio_neighbor_candidate(neighbor: &NetworkNeighbor) -> bool {
    if is_roborio_candidate_reachable(&neighbor.ip) {
        return true;
    }

    if !is_active_connection_neighbor_state(neighbor.state.as_deref()) {
        return false;
    }

    let has_roborio_mac = neighbor
        .mac
        .as_deref()
        .map(is_probable_roborio_mac)
        .unwrap_or(false);
    if !has_roborio_mac {
        return false;
    }

    // USB-IP neighbors are especially prone to stale ARP state after unplug.
    // Require endpoint reachability for 172.22.11.2-style links.
    if is_roborio_usb_ip(&neighbor.ip) {
        return false;
    }

    true
}

pub(crate) fn is_probable_roborio_ip(ip: &str) -> bool {
    let Ok(address) = ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let octets = address.octets();

    is_roborio_usb_ip(ip) || (octets[0] == 10 && octets[3] == 2)
}

pub(crate) fn is_probable_roborio_mac(mac: &str) -> bool {
    normalize_mac_address(mac).starts_with("00:80:2f")
}

pub(crate) fn merge_roborio_candidate(candidates: &mut Vec<NetworkNeighbor>, candidate: NetworkNeighbor) {
    if let Some(existing) = candidates.iter_mut().find(|entry| entry.ip == candidate.ip) {
        if existing.mac.is_none() {
            existing.mac = candidate.mac;
        }
        if existing.interface.is_none() {
            existing.interface = candidate.interface;
        }
        if existing.state.is_none() {
            existing.state = candidate.state;
        }
        return;
    }
    candidates.push(candidate);
}

pub(crate) fn roborio_neighbor_score(neighbor: &NetworkNeighbor, mdns_ips: &[String]) -> u16 {
    let is_usb = is_roborio_usb_link(&neighbor.ip, neighbor.interface.as_deref());
    let has_roborio_mac = neighbor
        .mac
        .as_deref()
        .map(is_probable_roborio_mac)
        .unwrap_or(false);
    let is_reachable_state = neighbor
        .state
        .as_deref()
        .map(|state| {
            matches!(
                state.to_ascii_uppercase().as_str(),
                "REACHABLE" | "STALE" | "DELAY" | "PROBE"
            )
        })
        .unwrap_or(false);
    let is_mdns = mdns_ips.iter().any(|ip| ip == &neighbor.ip);

    let mut score = 0_u16;
    if has_roborio_mac {
        score += 80;
    }
    if is_mdns {
        score += 100;
    }
    if is_usb {
        score += 40;
    }
    if is_reachable_state {
        score += 10;
    }
    if is_probable_roborio_ip(&neighbor.ip) {
        score += 5;
    }
    score
}

pub(crate) fn has_strong_roborio_signal_from_neighbor(neighbor: &NetworkNeighbor) -> bool {
    if is_roborio_usb_link(&neighbor.ip, neighbor.interface.as_deref()) {
        return true;
    }
    if neighbor
        .mac
        .as_deref()
        .map(is_probable_roborio_mac)
        .unwrap_or(false)
    {
        return true;
    }

    let interface = neighbor.interface.as_deref().unwrap_or_default();
    !interface.is_empty()
        && has_explicit_usb_interface_name(interface)
        && is_probable_roborio_ip(&neighbor.ip)
}

pub(crate) fn is_roborio_endpoint_responsive(ip: &str) -> bool {
    // Restrict to roboRIO-typical services so generic .2 hosts don't false-positive.
    let ports = [22_u16, 3580_u16, 80_u16];
    is_host_port_responsive(ip, &ports)
}

pub(crate) fn is_roborio_candidate_reachable(ip: &str) -> bool {
    is_roborio_endpoint_responsive(ip)
}

pub(crate) fn is_host_port_responsive(ip: &str, ports: &[u16]) -> bool {
    let Ok(address) = ip.parse::<Ipv4Addr>() else {
        return false;
    };

    for &port in ports {
        let socket = SocketAddr::V4(SocketAddrV4::new(address, port));
        if TcpStream::connect_timeout(
            &socket,
            Duration::from_millis(ROBOT_TCP_PROBE_TIMEOUT_MS),
        )
        .is_ok()
        {
            return true;
        }
    }

    false
}

pub(crate) fn is_roborio_usb_link(ip: &str, interface: Option<&str>) -> bool {
    if !is_roborio_usb_ip(ip) {
        return false;
    }

    if let Some(interface) = interface {
        if !should_ignore_interface(interface) && is_usb_network_interface(interface) {
            return true;
        }
    }

    let Some(resolved_interface) = resolve_interface_for_ip(ip) else {
        return false;
    };
    !should_ignore_interface(&resolved_interface) && is_usb_network_interface(&resolved_interface)
}

pub(crate) fn should_label_candidate_as_usb_link(
    ip: &str,
    interface: &str,
    usb_identity: Option<&UsbIdentity>,
) -> bool {
    if is_roborio_usb_ip(ip) {
        return true;
    }
    if usb_identity
        .map(is_expected_helios_usb_identity)
        .unwrap_or(false)
    {
        return true;
    }

    has_explicit_usb_interface_name(interface)
}
