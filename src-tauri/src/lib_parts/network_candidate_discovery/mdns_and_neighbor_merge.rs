use super::*;

pub(crate) fn discover_mdns_camera_candidates() -> Vec<IpDeviceCandidate> {
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

pub(crate) fn discover_neighbor_entries() -> Result<Vec<RawNeighborEntry>, String> {
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

pub(crate) fn is_inactive_neighbor_state(entry: &RawNeighborEntry) -> bool {
    matches!(
        entry.state.as_deref(),
        Some("FAILED") | Some("INCOMPLETE") | Some("NONE")
    )
}

pub(crate) fn probe_likely_frc_hosts() -> Vec<String> {
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

pub(crate) fn is_host_runtime_responsive(ip: &str) -> bool {
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

pub(crate) fn resolve_interface_for_ip(ip: &str) -> Option<String> {
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

pub(crate) fn dedupe_raw_neighbor_entries(entries: Vec<RawNeighborEntry>) -> Vec<RawNeighborEntry> {
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

pub(crate) fn merge_ip_candidate(candidates: &mut Vec<IpDeviceCandidate>, candidate: IpDeviceCandidate) {
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

pub(crate) fn merge_raw_neighbor_entry(current: &mut RawNeighborEntry, candidate: &RawNeighborEntry) {
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

pub(crate) fn candidate_neighbor_score(entry: &RawNeighborEntry) -> u8 {
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

