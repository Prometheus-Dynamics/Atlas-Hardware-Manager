use super::*;

pub(crate) fn discover_roborio_mdns_ips() -> Vec<String> {
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

pub(crate) fn discover_roborio_driver_station_or_local_ips() -> Vec<String> {
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

pub(crate) fn roborio_team_probe_ips() -> Vec<String> {
    infer_driver_station_team_numbers()
        .into_iter()
        .map(team_number_to_roborio_ip)
        .collect()
}

pub(crate) fn team_number_to_roborio_ip(team: u16) -> String {
    let major = team / 100;
    let minor = team % 100;
    format!("10.{major}.{minor}.2")
}

pub(crate) fn roborio_driver_station_hostnames() -> Vec<String> {
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

pub(crate) fn infer_driver_station_team_numbers() -> Vec<u16> {
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

pub(crate) fn infer_team_numbers_from_wpilib_preferences() -> Vec<u16> {
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

pub(crate) fn wpilib_preferences_candidate_paths() -> Vec<PathBuf> {
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

pub(crate) fn collect_team_numbers_from_text(text: &str, teams: &mut HashSet<u16>) {
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

pub(crate) fn collect_team_numbers_from_json(value: &Value, teams: &mut HashSet<u16>) {
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

pub(crate) fn extract_team_number_value(value: &Value) -> Option<u16> {
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

pub(crate) fn parse_team_number_token(value: &str) -> Option<u16> {
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

pub(crate) fn resolve_ipv4_hostname(hostname: &str) -> Option<String> {
    resolve_ipv4_hostname_with_timeout(
        hostname,
        Duration::from_millis(ROBORIO_HOSTNAME_RESOLVE_TIMEOUT_MS),
    )
}

pub(crate) fn resolve_ipv4_hostname_with_timeout(hostname: &str, timeout: Duration) -> Option<String> {
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

pub(crate) fn resolve_ipv4_hostname_blocking(hostname: &str) -> Option<String> {
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

pub(crate) fn is_roborio_usb_ip(ip: &str) -> bool {
    matches!(
        ip.parse::<Ipv4Addr>().ok().map(|address| address.octets()),
        Some([172, 22, 11, 2])
    )
}

pub(crate) fn discover_helios_ip_candidates() -> Result<Vec<IpDeviceCandidate>, String> {
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

pub(crate) fn is_probable_frc_robotics_ip(ip: &str) -> bool {
    is_probable_roborio_ip(ip) || is_probable_frc_vision_ip(ip)
}

pub(crate) fn is_probable_frc_vision_ip(ip: &str) -> bool {
    let Ok(address) = ip.parse::<Ipv4Addr>() else {
        return false;
    };
    let octets = address.octets();

    (11..=19).contains(&octets[3])
        && ((octets[0] == 10)
            || (octets[0] == 172 && octets[1] == 22 && octets[2] == 11)
            || (octets[0] == 169 && octets[1] == 254))
}

