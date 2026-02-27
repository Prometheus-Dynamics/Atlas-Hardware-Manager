fn format_voltage_value(value: String) -> String {
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

fn format_uptime_value_for_key(key: &str, value: &str) -> String {
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

fn format_duration_human(seconds: u64) -> String {
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

fn format_percentage_value(value: String) -> String {
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

fn format_bitrate_value(value: String) -> String {
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

fn parse_numeric_value(value: &str) -> Option<f64> {
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

fn format_usage_ratio_percentage(used: &str, total: &str) -> Option<String> {
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

fn select_likely_mounted_target(targets: &[FlashTarget]) -> Option<&FlashTarget> {
    let mut candidates = targets
        .iter()
        .filter(|target| is_likely_helios_mounted_target(target))
        .collect::<Vec<_>>();

    if candidates.len() == 1 {
        return candidates.pop();
    }

    None
}

fn is_likely_helios_mounted_target(target: &FlashTarget) -> bool {
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

const NEIGHBOR_PROBE_SETTLE_DELAY_MS: u64 = 70;
const ROBOT_TCP_PROBE_TIMEOUT_MS: u64 = 180;
const MAX_ROBORIO_ENDPOINT_PROBE_CANDIDATES: usize = 6;
const ROBORIO_HOSTNAME_RESOLVE_TIMEOUT_MS: u64 = 220;

fn discover_network_neighbors() -> Result<Vec<NetworkNeighbor>, String> {
    Ok(build_network_neighbors_from_raw_entries(
        discover_neighbor_entries()?,
    ))
}

fn discover_network_neighbors_fast() -> Result<Vec<NetworkNeighbor>, String> {
    let entries = dedupe_raw_neighbor_entries(collect_neighbor_entries());
    Ok(build_network_neighbors_from_raw_entries(entries))
}

fn build_network_neighbors_from_raw_entries(entries: Vec<RawNeighborEntry>) -> Vec<NetworkNeighbor> {
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

fn is_raspberry_pi_mac(mac: &str) -> bool {
    let normalized = mac.to_lowercase();
    let prefixes = ["b8:27:eb", "dc:a6:32", "e4:5f:01", "d8:3a:dd", "2c:cf:67"];
    prefixes.iter().any(|prefix| normalized.starts_with(prefix))
}

fn find_roborio_neighbor(neighbors: &[NetworkNeighbor]) -> Option<NetworkNeighbor> {
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

fn is_live_roborio_neighbor_candidate(neighbor: &NetworkNeighbor) -> bool {
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

fn is_probable_roborio_ip(ip: &str) -> bool {
    let Ok(address) = ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let octets = address.octets();

    is_roborio_usb_ip(ip) || (octets[0] == 10 && octets[3] == 2)
}

fn is_probable_roborio_mac(mac: &str) -> bool {
    normalize_mac_address(mac).starts_with("00:80:2f")
}

fn merge_roborio_candidate(candidates: &mut Vec<NetworkNeighbor>, candidate: NetworkNeighbor) {
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

fn roborio_neighbor_score(neighbor: &NetworkNeighbor, mdns_ips: &[String]) -> u16 {
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

fn has_strong_roborio_signal_from_neighbor(neighbor: &NetworkNeighbor) -> bool {
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

fn is_roborio_endpoint_responsive(ip: &str) -> bool {
    // Restrict to roboRIO-typical services so generic .2 hosts don't false-positive.
    let ports = [22_u16, 3580_u16, 80_u16];
    is_host_port_responsive(ip, &ports)
}

fn is_roborio_candidate_reachable(ip: &str) -> bool {
    is_roborio_endpoint_responsive(ip)
}

fn is_host_port_responsive(ip: &str, ports: &[u16]) -> bool {
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

fn is_roborio_usb_link(ip: &str, interface: Option<&str>) -> bool {
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

fn should_label_candidate_as_usb_link(
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

fn discover_roborio_mdns_ips() -> Vec<String> {
    let Some(avahi_browse_path) = find_in_path("avahi-browse") else {
        return Vec::new();
    };

    let Ok(output) = Command::new(avahi_browse_path)
        .args(["-a", "-r", "-p", "-t"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut ips = HashSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('=') {
            continue;
        }
        let parts = trimmed.split(';').collect::<Vec<_>>();
        if parts.len() < 8 {
            continue;
        }

        let service_name = parts[3].trim().to_ascii_lowercase();
        let service_type = parts[4].trim().to_ascii_lowercase();
        let host_name = parts[6].trim().to_ascii_lowercase();
        let is_roborio_named = service_name.contains("roborio") || host_name.contains("roborio");
        let is_ni_service = matches!(
            service_type.as_str(),
            "_ni._tcp" | "_ni-sysapi._tcp" | "_ni-rt._tcp"
        );
        if !(is_roborio_named || is_ni_service) {
            continue;
        }

        let ip = parts[7].trim();
        if ip.is_empty() || ip.contains(':') {
            continue;
        }
        ips.insert(ip.to_string());
    }

    let mut values = ips.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn discover_roborio_driver_station_or_local_ips() -> Vec<String> {
    let inferred_targets = likely_robot_probe_targets()
        .into_iter()
        .chain(roborio_team_probe_ips())
        .filter(|ip| is_probable_roborio_ip(ip))
        .collect::<HashSet<_>>();
    let mut hostname_targets = HashSet::new();

    for hostname in roborio_driver_station_hostnames() {
        if let Some(ip) = resolve_ipv4_hostname(&hostname) {
            hostname_targets.insert(ip);
        }
    }

    let targets = inferred_targets
        .union(&hostname_targets)
        .cloned()
        .collect::<HashSet<_>>();
    if targets.is_empty() {
        return Vec::new();
    }

    let mut workers = Vec::with_capacity(targets.len());
    for ip in targets {
        let inferred_only = inferred_targets.contains(&ip) && !hostname_targets.contains(&ip);
        workers.push(thread::spawn(move || {
            let accepted = if inferred_only {
                // For guessed local/DS IPs, require roboRIO-like reachability.
                is_roborio_candidate_reachable(&ip)
            } else {
                // Hostname-derived candidates can use either roboRIO service or web endpoint.
                is_roborio_candidate_reachable(&ip)
                    || is_host_port_responsive(&ip, &[80_u16, 443_u16])
            };

            if accepted {
                Some(ip)
            } else {
                None
            }
        }));
    }

    let mut hits = Vec::new();
    for worker in workers {
        if let Ok(Some(ip)) = worker.join() {
            hits.push(ip);
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

fn roborio_team_probe_ips() -> Vec<String> {
    infer_driver_station_team_numbers()
        .into_iter()
        .map(team_number_to_roborio_ip)
        .collect()
}

fn team_number_to_roborio_ip(team: u16) -> String {
    let major = team / 100;
    let minor = team % 100;
    format!("10.{major}.{minor}.2")
}

fn roborio_driver_station_hostnames() -> Vec<String> {
    let mut hostnames = HashSet::new();
    hostnames.insert("roborio.local".to_string());

    for team in infer_driver_station_team_numbers() {
        hostnames.insert(format!("roborio-{team}-frc.local"));
        hostnames.insert(format!("roborio-{team}.local"));
    }

    let mut values = hostnames.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn infer_driver_station_team_numbers() -> Vec<u16> {
    let mut teams = HashSet::new();

    for address in discover_local_ipv4_addresses() {
        let octets = address.octets();
        if octets[0] != 10 {
            continue;
        }

        let team = (octets[1] as u16) * 100 + octets[2] as u16;
        if (1..=9999).contains(&team) {
            teams.insert(team);
        }
    }
    for team in infer_team_numbers_from_wpilib_preferences() {
        teams.insert(team);
    }

    let mut values = teams.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn infer_team_numbers_from_wpilib_preferences() -> Vec<u16> {
    let mut teams = HashSet::new();
    for path in wpilib_preferences_candidate_paths() {
        if !path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        collect_team_numbers_from_text(&text, &mut teams);
    }

    let mut values = teams.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn wpilib_preferences_candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(home) = env::var_os("HOME") {
        let home_path = PathBuf::from(home);
        paths.push(home_path.join(".wpilib").join("wpilib_preferences.json"));
        paths.push(home_path.join("wpilib_preferences.json"));
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        let profile_path = PathBuf::from(profile);
        paths.push(profile_path.join(".wpilib").join("wpilib_preferences.json"));
        paths.push(profile_path.join("wpilib_preferences.json"));
    }
    if let Some(appdata) = env::var_os("APPDATA") {
        paths.push(PathBuf::from(appdata).join("WPILib").join("wpilib_preferences.json"));
    }
    if let Some(local_appdata) = env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local_appdata).join("WPILib").join("wpilib_preferences.json"));
    }

    let mut unique = HashSet::new();
    paths
        .into_iter()
        .filter(|path| unique.insert(path.to_string_lossy().to_string()))
        .collect()
}

fn collect_team_numbers_from_text(text: &str, teams: &mut HashSet<u16>) {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        collect_team_numbers_from_json(&value, teams);
    }

    let lower = text.to_ascii_lowercase();
    for marker in ["teamnumber", "team_number", "\"team\""] {
        let mut cursor = 0usize;
        while let Some(relative_index) = lower[cursor..].find(marker) {
            let index = cursor + relative_index + marker.len();
            let search_end = (index + 24).min(lower.len());
            let window = &lower[index..search_end];
            for token in window.split(|character: char| !character.is_ascii_digit()) {
                if let Some(team) = parse_team_number_token(token) {
                    teams.insert(team);
                }
            }
            cursor = index;
            if cursor >= lower.len() {
                break;
            }
        }
    }
}

fn collect_team_numbers_from_json(value: &Value, teams: &mut HashSet<u16>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized_key = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .map(|character| character.to_ascii_lowercase())
                    .collect::<String>();
                if normalized_key.contains("team") {
                    if let Some(team) = extract_team_number_value(child) {
                        teams.insert(team);
                    }
                }
                collect_team_numbers_from_json(child, teams);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_team_numbers_from_json(item, teams);
            }
        }
        _ => {}
    }
}

fn extract_team_number_value(value: &Value) -> Option<u16> {
    if let Some(number) = value.as_u64() {
        return parse_team_number_token(&number.to_string());
    }
    if let Some(number) = value.as_i64() {
        if number >= 0 {
            return parse_team_number_token(&number.to_string());
        }
    }
    value.as_str().and_then(parse_team_number_token)
}

fn parse_team_number_token(value: &str) -> Option<u16> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 4 {
        return None;
    }
    let parsed = trimmed.parse::<u16>().ok()?;
    if (1..=9999).contains(&parsed) {
        Some(parsed)
    } else {
        None
    }
}

fn resolve_ipv4_hostname(hostname: &str) -> Option<String> {
    resolve_ipv4_hostname_with_timeout(
        hostname,
        Duration::from_millis(ROBORIO_HOSTNAME_RESOLVE_TIMEOUT_MS),
    )
}

fn resolve_ipv4_hostname_with_timeout(hostname: &str, timeout: Duration) -> Option<String> {
    let target = hostname.trim().to_string();
    if target.is_empty() {
        return None;
    }

    let (sender, receiver) = std::sync::mpsc::channel::<Option<String>>();
    thread::spawn(move || {
        let _ = sender.send(resolve_ipv4_hostname_blocking(&target));
    });

    receiver.recv_timeout(timeout).ok().flatten()
}

fn resolve_ipv4_hostname_blocking(hostname: &str) -> Option<String> {
    let target = format!("{hostname}:80");
    let addresses = target.to_socket_addrs().ok()?;
    for address in addresses {
        let SocketAddr::V4(v4_address) = address else {
            continue;
        };
        if v4_address.ip().is_loopback() {
            continue;
        }
        return Some(v4_address.ip().to_string());
    }

    None
}

fn is_roborio_usb_ip(ip: &str) -> bool {
    matches!(
        ip.parse::<Ipv4Addr>().ok().map(|address| address.octets()),
        Some([172, 22, 11, 2])
    )
}

fn discover_helios_ip_candidates() -> Result<Vec<IpDeviceCandidate>, String> {
    let mut candidates = Vec::new();
    for entry in discover_neighbor_entries()? {
        if entry.ip.contains(':') {
            continue;
        }

        let interface = entry.interface.unwrap_or_else(|| "unknown".to_string());
        if interface != "unknown" && should_ignore_interface(&interface) {
            continue;
        }

        if matches!(
            entry.state.as_deref(),
            Some("FAILED") | Some("INCOMPLETE") | Some("NONE")
        ) {
            continue;
        }

        let mac = entry.mac;
        let usb_identity = if interface != "unknown" {
            lookup_usb_identity_for_interface(&interface)
        } else {
            None
        };
        let is_usb_link = interface != "unknown"
            && should_label_candidate_as_usb_link(&entry.ip, &interface, usb_identity.as_ref());
        let is_helios_usb = usb_identity
            .as_ref()
            .map(is_expected_helios_usb_identity)
            .unwrap_or(false);
        let is_rpi_mac = mac.as_deref().map(is_raspberry_pi_mac).unwrap_or(false);
        let is_frc_robotics_ip = is_probable_frc_robotics_ip(&entry.ip);

        if !(is_helios_usb || is_rpi_mac || is_frc_robotics_ip) {
            continue;
        }

        candidates.push(IpDeviceCandidate {
            ip: entry.ip,
            mac,
            interface,
            state: entry.state,
            is_usb_link,
            usb_identity,
        });
    }

    for mdns_candidate in discover_mdns_camera_candidates() {
        merge_ip_candidate(&mut candidates, mdns_candidate);
    }

    candidates.sort_by(|left, right| left.ip.cmp(&right.ip));
    Ok(candidates)
}

fn is_probable_frc_robotics_ip(ip: &str) -> bool {
    is_probable_roborio_ip(ip) || is_probable_frc_vision_ip(ip)
}

fn is_probable_frc_vision_ip(ip: &str) -> bool {
    let Ok(address) = ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let octets = address.octets();

    (11..=19).contains(&octets[3])
        && ((octets[0] == 10)
            || (octets[0] == 172 && octets[1] == 22 && octets[2] == 11)
            || (octets[0] == 169 && octets[1] == 254))
}

fn discover_mdns_camera_candidates() -> Vec<IpDeviceCandidate> {
    let Some(avahi_browse_path) = find_in_path("avahi-browse") else {
        return Vec::new();
    };

    let Ok(output) = Command::new(avahi_browse_path)
        .args(["-a", "-r", "-p", "-t"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('=') {
            continue;
        }

        let parts = trimmed.split(';').collect::<Vec<_>>();
        if parts.len() < 8 {
            continue;
        }

        let interface = parts[1].trim();
        if interface.is_empty() || should_ignore_interface(interface) {
            continue;
        }

        let service_name = parts[3].trim();
        let service_type = parts[4].trim();
        let host_name = parts[6].trim();
        let ip = parts[7].trim();
        if ip.is_empty() || ip.contains(':') {
            continue;
        }

        let probe_text = format!("{service_name} {service_type} {host_name}").to_ascii_lowercase();
        let is_camera_service = probe_text.contains("limelight")
            || probe_text.contains("photonvision")
            || service_type.eq_ignore_ascii_case("_http._tcp")
                && (probe_text.contains("photon") || probe_text.contains("vision"));
        if !is_camera_service {
            continue;
        }

        let usb_identity = lookup_usb_identity_for_interface(interface);
        let is_usb_link = should_label_candidate_as_usb_link(ip, interface, usb_identity.as_ref());
        let resolved_interface = if interface.is_empty() {
            resolve_interface_for_ip(ip).unwrap_or_else(|| "unknown".to_string())
        } else {
            interface.to_string()
        };

        merge_ip_candidate(
            &mut candidates,
            IpDeviceCandidate {
                ip: ip.to_string(),
                mac: None,
                interface: resolved_interface,
                state: Some("REACHABLE".to_string()),
                is_usb_link,
                usb_identity,
            },
        );
    }

    candidates
}

fn discover_neighbor_entries() -> Result<Vec<RawNeighborEntry>, String> {
    let mut entries = collect_neighbor_entries();
    if entries.is_empty() {
        prime_neighbor_cache_with_likely_robot_ips();
        entries = collect_neighbor_entries();
    }

    let has_active_vision_neighbor = entries
        .iter()
        .any(|entry| is_probable_frc_vision_ip(&entry.ip) && !is_inactive_neighbor_state(entry));
    if !has_active_vision_neighbor {
        prime_neighbor_cache_for_local_frc_subnets(&entries);
        entries.extend(collect_neighbor_entries());
    }

    let has_active_vision_neighbor = entries
        .iter()
        .any(|entry| is_probable_frc_vision_ip(&entry.ip) && !is_inactive_neighbor_state(entry));
    if !has_active_vision_neighbor {
        let probe_hits = probe_likely_frc_hosts();
        if !probe_hits.is_empty() {
            thread::sleep(Duration::from_millis(NEIGHBOR_PROBE_SETTLE_DELAY_MS));
            entries.extend(collect_neighbor_entries());

            let known_ips = entries.iter().map(|entry| entry.ip.clone()).collect::<HashSet<_>>();
            for ip in probe_hits {
                if known_ips.contains(&ip) {
                    continue;
                }
                entries.push(RawNeighborEntry {
                    ip: ip.clone(),
                    mac: None,
                    interface: resolve_interface_for_ip(&ip),
                    state: Some("REACHABLE".to_string()),
                });
            }
        }
    }

    Ok(dedupe_raw_neighbor_entries(entries))
}

fn is_inactive_neighbor_state(entry: &RawNeighborEntry) -> bool {
    matches!(
        entry.state.as_deref(),
        Some("FAILED") | Some("INCOMPLETE") | Some("NONE")
    )
}

fn probe_likely_frc_hosts() -> Vec<String> {
    let targets = likely_robot_probe_targets()
        .into_iter()
        .filter(|ip| is_probable_frc_robotics_ip(ip))
        .collect::<Vec<_>>();

    if targets.is_empty() {
        return Vec::new();
    }

    let mut workers = Vec::with_capacity(targets.len());
    for ip in targets {
        workers.push(thread::spawn(move || {
            if is_host_runtime_responsive(&ip) {
                Some(ip)
            } else {
                None
            }
        }));
    }

    let mut hits = Vec::new();
    for worker in workers {
        if let Ok(Some(ip)) = worker.join() {
            hits.push(ip);
        }
    }
    hits.sort();
    hits
}

fn is_host_runtime_responsive(ip: &str) -> bool {
    let Ok(address) = ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let timeout = Duration::from_millis(ROBOT_TCP_PROBE_TIMEOUT_MS);
    let ports = [5800_u16, 5801_u16, 22_u16, 3580_u16, 80_u16, 443_u16];

    for port in ports {
        let socket = SocketAddr::V4(SocketAddrV4::new(address, port));
        match TcpStream::connect_timeout(&socket, timeout) {
            Ok(stream) => {
                let _ = stream.set_read_timeout(Some(timeout));
                let _ = stream.set_write_timeout(Some(timeout));
                return true;
            }
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                return true;
            }
            Err(_) => {}
        }
    }

    false
}

fn resolve_interface_for_ip(ip: &str) -> Option<String> {
    let ip_path = find_in_path("ip")?;
    let output = Command::new(ip_path)
        .args(["-j", "route", "get", ip])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value = serde_json::from_slice::<Value>(&output.stdout).ok()?;
    let routes = value.as_array()?;
    let route = routes.first()?;
    let interface = route.get("dev").and_then(Value::as_str)?.trim();
    if interface.is_empty() {
        None
    } else {
        Some(interface.to_string())
    }
}

fn dedupe_raw_neighbor_entries(entries: Vec<RawNeighborEntry>) -> Vec<RawNeighborEntry> {
    let mut by_ip: HashMap<String, RawNeighborEntry> = HashMap::new();

    for entry in entries {
        by_ip
            .entry(entry.ip.clone())
            .and_modify(|current| merge_raw_neighbor_entry(current, &entry))
            .or_insert(entry);
    }

    let mut values = by_ip.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| left.ip.cmp(&right.ip));
    values
}

fn merge_ip_candidate(candidates: &mut Vec<IpDeviceCandidate>, candidate: IpDeviceCandidate) {
    if let Some(existing) = candidates.iter_mut().find(|entry| entry.ip == candidate.ip) {
        if existing.mac.is_none() {
            existing.mac = candidate.mac;
        }
        if existing.interface == "unknown" && candidate.interface != "unknown" {
            existing.interface = candidate.interface.clone();
        }
        if existing.state.is_none() {
            existing.state = candidate.state;
        }
        let merged_interface = if existing.interface != "unknown" {
            existing.interface.clone()
        } else {
            candidate.interface.clone()
        };
        if existing.usb_identity.is_none() {
            existing.usb_identity = candidate.usb_identity;
        }
        existing.is_usb_link = merged_interface != "unknown"
            && should_label_candidate_as_usb_link(
                &existing.ip,
                &merged_interface,
                existing.usb_identity.as_ref(),
            );
        return;
    }

    candidates.push(candidate);
}

fn merge_raw_neighbor_entry(current: &mut RawNeighborEntry, candidate: &RawNeighborEntry) {
    if candidate_neighbor_score(candidate) > candidate_neighbor_score(current) {
        current.state = candidate.state.clone();
    }
    if current.mac.is_none() {
        current.mac = candidate.mac.clone();
    }
    if current.interface.is_none() {
        current.interface = candidate.interface.clone();
    }
}

fn candidate_neighbor_score(entry: &RawNeighborEntry) -> u8 {
    let state_score = match entry.state.as_deref().unwrap_or_default() {
        "REACHABLE" | "DELAY" | "PROBE" | "STALE" => 4,
        "PERMANENT" => 3,
        "INCOMPLETE" | "FAILED" | "NONE" => 0,
        _ => 2,
    };
    let mac_score = if entry.mac.is_some() { 2 } else { 0 };
    let interface_score = if entry.interface.is_some() { 1 } else { 0 };
    state_score + mac_score + interface_score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_number_maps_to_roborio_ip() {
        assert_eq!(team_number_to_roborio_ip(6390), "10.63.90.2");
        assert_eq!(team_number_to_roborio_ip(254), "10.2.54.2");
    }

    #[test]
    fn usb_ip_neighbor_requires_real_reachability() {
        let neighbor = NetworkNeighbor {
            ip: "172.22.11.2".to_string(),
            mac: Some("00:80:2f:aa:bb:cc".to_string()),
            interface: Some("usb0".to_string()),
            state: Some("REACHABLE".to_string()),
            is_helios_candidate: false,
        };

        assert!(!is_live_roborio_neighbor_candidate(&neighbor));
    }

    #[test]
    fn active_neighbor_with_ni_mac_can_still_be_live() {
        let neighbor = NetworkNeighbor {
            ip: "10.63.90.2".to_string(),
            mac: Some("00:80:2f:11:22:33".to_string()),
            interface: Some("eth0".to_string()),
            state: Some("REACHABLE".to_string()),
            is_helios_candidate: false,
        };

        assert!(is_live_roborio_neighbor_candidate(&neighbor));
    }

    #[test]
    fn incomplete_neighbor_without_mac_is_not_selected() {
        let neighbors = vec![NetworkNeighbor {
            ip: "10.63.90.2".to_string(),
            mac: None,
            interface: Some("enp0s1".to_string()),
            state: Some("INCOMPLETE".to_string()),
            is_helios_candidate: false,
        }];

        assert!(find_roborio_neighbor(&neighbors).is_none());
    }
}
