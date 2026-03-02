#[path = "network_neighbor_parsing/local_ip_discovery.rs"]
mod local_ip_discovery;
pub(crate) use local_ip_discovery::*;

fn discover_neighbor_entries_from_ip() -> Result<Vec<RawNeighborEntry>, String> {
    let Some(ip_path) = find_in_path("ip") else {
        return Ok(Vec::new());
    };

    let output = Command::new(ip_path)
        .args(["-j", "neigh"])
        .output()
        .map_err(|error| format!("Failed to execute ip neigh: {error}"))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => return Ok(Vec::new()),
    };

    let mut entries = Vec::new();
    if let Some(array) = value.as_array() {
        for entry in array {
            let Some(ip) = entry.get("dst").and_then(Value::as_str) else {
                continue;
            };
            let state = parse_neighbor_state(entry.get("state"));
            let mac = entry
                .get("lladdr")
                .and_then(Value::as_str)
                .map(normalize_mac_address);
            let interface = entry
                .get("dev")
                .and_then(Value::as_str)
                .map(|value| value.to_string());

            entries.push(RawNeighborEntry {
                ip: ip.to_string(),
                mac,
                interface,
                state,
            });
        }
    }

    Ok(entries)
}

fn discover_neighbor_entries_from_arp() -> Result<Vec<RawNeighborEntry>, String> {
    let Some(arp_path) = find_in_path("arp") else {
        return Ok(Vec::new());
    };

    let args = if cfg!(target_os = "windows") {
        vec!["-a"]
    } else {
        vec!["-an"]
    };

    let output = Command::new(arp_path)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to execute arp: {error}"))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut current_interface = None::<String>;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.to_ascii_lowercase().starts_with("interface:") {
            current_interface = extract_first_ipv4(trimmed).map(|ip| format!("iface-{ip}"));
            continue;
        }

        let Some(ip) = extract_first_ipv4(trimmed) else {
            continue;
        };

        let mac = extract_first_mac(trimmed).map(normalize_mac_address);
        let interface =
            extract_interface_from_arp_line(trimmed).or_else(|| current_interface.clone());
        let state = Some(if trimmed.to_ascii_lowercase().contains("incomplete") {
            "INCOMPLETE".to_string()
        } else {
            "REACHABLE".to_string()
        });

        entries.push(RawNeighborEntry {
            ip,
            mac,
            interface,
            state,
        });
    }

    Ok(entries)
}

fn parse_neighbor_state(value: Option<&Value>) -> Option<String> {
    let value = value?;

    if let Some(text) = value.as_str() {
        return Some(text.to_ascii_uppercase());
    }

    if let Some(items) = value.as_array() {
        for item in items {
            if let Some(text) = item.as_str() {
                return Some(text.to_ascii_uppercase());
            }
        }
    }

    None
}

fn extract_first_ipv4(line: &str) -> Option<String> {
    line.split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find_map(|token| {
            if token.is_empty() {
                return None;
            }
            token
                .parse::<Ipv4Addr>()
                .ok()
                .map(|address| address.to_string())
        })
}

fn extract_first_mac(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let trimmed = token.trim_matches(|character: char| {
            character == '(' || character == ')' || character == ',' || character == ';'
        });
        let normalized = normalize_mac_address(trimmed);
        if is_mac_address(&normalized) {
            Some(normalized)
        } else {
            None
        }
    })
}

fn extract_interface_from_arp_line(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let marker = " on ";
    let index = lower.find(marker)?;
    let remainder = line[index + marker.len()..].trim();
    let interface = remainder
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim();
    if interface.is_empty() {
        None
    } else {
        Some(interface.to_string())
    }
}

fn normalize_mac_address(mac: impl AsRef<str>) -> String {
    mac.as_ref().trim().replace('-', ":").to_ascii_lowercase()
}

fn is_mac_address(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 6 {
        return false;
    }
    parts
        .iter()
        .all(|part| part.len() == 2 && part.chars().all(|character| character.is_ascii_hexdigit()))
}

fn should_ignore_interface(interface: &str) -> bool {
    let ignored_prefixes = [
        "lo",
        "docker",
        "br-",
        "veth",
        "virbr",
        "tailscale",
        "zt",
        "wg",
        "tun",
        "tap",
    ];
    ignored_prefixes
        .iter()
        .any(|prefix| interface.starts_with(prefix))
}

fn is_usb_network_interface(interface: &str) -> bool {
    let lowered = interface.to_ascii_lowercase();
    if lowered.contains("usb") || lowered.contains("rndis") {
        return true;
    }

    if !cfg!(target_os = "linux") {
        return false;
    }

    let device_path = Path::new("/sys/class/net").join(interface).join("device");
    let Ok(canonical) = fs::canonicalize(device_path) else {
        return false;
    };

    canonical
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("/usb")
}

fn has_explicit_usb_interface_name(interface: &str) -> bool {
    let lowered = interface.trim().to_ascii_lowercase();
    lowered.starts_with("usb")
        || lowered.contains("rndis")
        || lowered.contains("gadget")
        || lowered.contains("ncm")
}

fn lookup_usb_identity_for_interface(interface: &str) -> Option<UsbIdentity> {
    let device_path = Path::new("/sys/class/net").join(interface).join("device");
    let mut current = fs::canonicalize(device_path).ok()?;

    loop {
        let id_vendor = read_trimmed_file(current.join("idVendor"));
        let id_product = read_trimmed_file(current.join("idProduct"));
        if let (Some(vendor_id), Some(product_id)) = (id_vendor, id_product) {
            return Some(UsbIdentity {
                vendor_id: vendor_id.to_ascii_lowercase(),
                product_id: product_id.to_ascii_lowercase(),
                manufacturer: read_trimmed_file(current.join("manufacturer")),
                product: read_trimmed_file(current.join("product")),
                serial: read_trimmed_file(current.join("serial")),
            });
        }

        if !current.pop() {
            break;
        }
    }

    None
}

fn read_trimmed_file(path: PathBuf) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_expected_helios_usb_identity(identity: &UsbIdentity) -> bool {
    if HELIOS_USB_ID_ALLOWLIST
        .iter()
        .any(|(vendor, product)| identity.vendor_id == *vendor && identity.product_id == *product)
    {
        return true;
    }

    let manufacturer = identity
        .manufacturer
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let product = identity
        .product
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let serial = identity
        .serial
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    manufacturer.contains("helios")
        || product.contains("helios")
        || product.contains("hvs")
        || serial.starts_with("helios")
}

fn discovery_rank(status: &str, chips: &[String]) -> u8 {
    if status == "bootloader" {
        return 0;
    }
    if status == "mounted" {
        return 1;
    }
    if chips.iter().any(|chip| chip == "USB IP") {
        return 2;
    }
    3
}

pub(crate) fn resolve_usb_topology_path_for_interface(interface: &str) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let trimmed_interface = interface.trim();
    if trimmed_interface.is_empty() {
        return None;
    }

    let device_path = Path::new("/sys/class/net")
        .join(trimmed_interface)
        .join("device");
    let canonical = fs::canonicalize(device_path).ok()?;
    resolve_usb_topology_path_from_sys_path(canonical)
}

fn resolve_usb_topology_path_for_block_device(device_path: &str) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let trimmed_path = device_path.trim();
    if trimmed_path.is_empty() {
        return None;
    }

    let block_name = Path::new(trimmed_path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let mut block_candidates = vec![block_name.to_string()];
    let trimmed_digits = block_name.trim_end_matches(|character: char| character.is_ascii_digit());
    if trimmed_digits != block_name {
        let mut candidate = trimmed_digits.to_string();
        if candidate.ends_with('p') {
            candidate.pop();
        }
        if !candidate.is_empty() && candidate != block_name {
            block_candidates.push(candidate);
        }
    }

    for candidate in block_candidates {
        let canonical = match fs::canonicalize(Path::new("/sys/class/block").join(&candidate)) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if let Some(usb_path) = resolve_usb_topology_path_from_sys_path(canonical) {
            return Some(usb_path);
        }
    }

    None
}

fn resolve_usb_topology_path_from_sys_path(mut path: PathBuf) -> Option<String> {
    loop {
        if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            if let Some(usb_path) = normalize_usb_topology_path(name) {
                return Some(usb_path);
            }
        }
        if !path.pop() {
            break;
        }
    }
    None
}

fn resolve_usb_topology_path_from_text(text: &str) -> Option<String> {
    text.split(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '·' | '|' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '\\'
            )
    })
    .find_map(normalize_usb_topology_path)
}

fn normalize_usb_topology_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && !matches!(character, '-' | '.' | ':')
    });
    if trimmed.is_empty() {
        return None;
    }

    let without_suffix = trimmed.split(':').next().unwrap_or(trimmed).trim();
    let (bus_raw, ports_raw) = without_suffix.split_once('-')?;
    if bus_raw.is_empty() || ports_raw.is_empty() {
        return None;
    }
    if !bus_raw.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    let mut normalized_ports = Vec::new();
    for segment in ports_raw.split('.') {
        if segment.is_empty() || !segment.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        let parsed = segment.parse::<u16>().ok()?;
        normalized_ports.push(parsed.to_string());
    }
    if normalized_ports.is_empty() {
        return None;
    }

    let bus = bus_raw.parse::<u16>().ok()?;
    Some(format!("{bus}-{}", normalized_ports.join(".")))
}

fn resolve_device_usb_topology_path(device: &DiscoveredDevice) -> Option<String> {
    if let Some(usb_location) = device.usb_location.as_deref() {
        if let Some(path) = resolve_usb_topology_path_from_text(usb_location) {
            return Some(path);
        }
        if let Some(path) = resolve_usb_topology_path_for_block_device(usb_location) {
            return Some(path);
        }
    }

    device
        .interface_name
        .as_deref()
        .and_then(resolve_usb_topology_path_for_interface)
}

fn is_bootloader_or_mounted_status(status: &str) -> bool {
    matches!(status, "bootloader" | "mounted")
}

fn is_live_usb_device(device: &DiscoveredDevice) -> bool {
    if device.status != "online" {
        return false;
    }

    if device
        .connection_chips
        .iter()
        .any(|chip| chip.eq_ignore_ascii_case("USB IP"))
    {
        return true;
    }

    device
        .interface_name
        .as_deref()
        .map(|interface| {
            !should_ignore_interface(interface) && is_usb_network_interface(interface)
        })
        .unwrap_or(false)
}

fn usb_path_preference_score(device: &DiscoveredDevice) -> i32 {
    let mut score = match device.status.as_str() {
        "online" => 300,
        "mounted" => 200,
        "bootloader" => 100,
        _ => 0,
    };

    if device
        .connection_chips
        .iter()
        .any(|chip| chip.eq_ignore_ascii_case("USB IP"))
    {
        score += 40;
    }
    if device
        .runtime_product
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        score += 20;
    }
    if device
        .ip_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        score += 10;
    }

    score
}

fn select_preferred_usb_path_device(
    indexes: &[usize],
    devices: &[DiscoveredDevice],
) -> Option<usize> {
    indexes.iter().copied().max_by(|left, right| {
        usb_path_preference_score(&devices[*left])
            .cmp(&usb_path_preference_score(&devices[*right]))
            .then_with(|| devices[*left].display_name.cmp(&devices[*right].display_name))
            .then_with(|| devices[*left].id.cmp(&devices[*right].id))
    })
}

fn prune_usb_port_stale_devices(devices: &mut Vec<DiscoveredDevice>) {
    let mut indexes_by_usb_path: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, device) in devices.iter().enumerate() {
        let Some(usb_path) = resolve_device_usb_topology_path(device) else {
            continue;
        };
        indexes_by_usb_path.entry(usb_path).or_default().push(index);
    }

    let mut remove_indexes = HashSet::new();
    for indexes in indexes_by_usb_path.into_values() {
        if indexes.len() < 2 {
            continue;
        }

        let online_indexes = indexes
            .iter()
            .copied()
            .filter(|index| is_live_usb_device(&devices[*index]))
            .collect::<Vec<_>>();

        if !online_indexes.is_empty() {
            let Some(keep_index) = select_preferred_usb_path_device(&online_indexes, devices) else {
                continue;
            };
            for index in indexes {
                if index == keep_index {
                    continue;
                }
                if is_bootloader_or_mounted_status(devices[index].status.as_str())
                    || is_live_usb_device(&devices[index])
                {
                    remove_indexes.insert(index);
                }
            }
            continue;
        }

        let transitional_indexes = indexes
            .iter()
            .copied()
            .filter(|index| is_bootloader_or_mounted_status(devices[*index].status.as_str()))
            .collect::<Vec<_>>();
        if transitional_indexes.len() < 2 {
            continue;
        }

        let Some(keep_index) = select_preferred_usb_path_device(&transitional_indexes, devices)
        else {
            continue;
        };
        for index in transitional_indexes {
            if index != keep_index {
                remove_indexes.insert(index);
            }
        }
    }

    if remove_indexes.is_empty() {
        return;
    }

    let mut pruned = Vec::with_capacity(devices.len().saturating_sub(remove_indexes.len()));
    for (index, device) in devices.drain(..).enumerate() {
        if !remove_indexes.contains(&index) {
            pruned.push(device);
        }
    }
    *devices = pruned;
}

fn dedupe_discovered_devices(devices: &mut Vec<DiscoveredDevice>) {
    let mut merged: Vec<DiscoveredDevice> = Vec::new();

    for device in devices.drain(..) {
        let mut found = false;
        for existing in &mut merged {
            let same_mac = existing.mac_address.is_some()
                && device.mac_address.is_some()
                && existing.mac_address == device.mac_address;
            let same_ip = existing.ip_address.is_some()
                && device.ip_address.is_some()
                && existing.ip_address == device.ip_address;
            if !(same_mac || same_ip) {
                continue;
            }

            for chip in &device.connection_chips {
                if !existing.connection_chips.contains(chip) {
                    existing.connection_chips.push(chip.clone());
                }
            }

            if existing.ip_address.is_none() {
                existing.ip_address = device.ip_address.clone();
            }
            if existing.interface_name.is_none() {
                existing.interface_name = device.interface_name.clone();
            }
            if existing.usb_location.is_none() {
                existing.usb_location = device.usb_location.clone();
            }
            if existing.vendor_product.is_none() {
                existing.vendor_product = device.vendor_product.clone();
            }
            if existing.runtime_product.is_none() {
                existing.runtime_product = device.runtime_product.clone();
            }
            if existing.firmware_version.is_none() {
                existing.firmware_version = device.firmware_version.clone();
            }
            if existing.os_version.is_none() {
                existing.os_version = device.os_version.clone();
            }
            if existing.telemetry_summary.is_none() {
                existing.telemetry_summary = device.telemetry_summary.clone();
            }
            if should_prefer_display_name(&existing.display_name, &device.display_name) {
                existing.display_name = device.display_name.clone();
                if !device.detail.trim().is_empty() {
                    existing.detail = device.detail.clone();
                }
            }
            if existing.detail.trim().is_empty() && !device.detail.trim().is_empty() {
                existing.detail = device.detail.clone();
            }

            found = true;
            break;
        }

        if !found {
            merged.push(device);
        }
    }

    for device in &mut merged {
        device.connection_chips.sort();
        device.connection_chips.dedup();
    }
    prune_usb_port_stale_devices(&mut merged);

    *devices = merged;
}

fn should_prefer_display_name(current: &str, incoming: &str) -> bool {
    let current_trimmed = current.trim();
    let incoming_trimmed = incoming.trim();
    if incoming_trimmed.is_empty() {
        return false;
    }
    if current_trimmed.is_empty() {
        return true;
    }
    if current_trimmed.eq_ignore_ascii_case("unknown device")
        && !incoming_trimmed.eq_ignore_ascii_case("unknown device")
    {
        return true;
    }
    if current_trimmed.eq_ignore_ascii_case("unclassified device")
        && !incoming_trimmed.eq_ignore_ascii_case("unclassified device")
    {
        return true;
    }

    let current_generic = is_generic_device_display_name(current_trimmed);
    let incoming_generic = is_generic_device_display_name(incoming_trimmed);
    if current_generic && !incoming_generic {
        return true;
    }
    if looks_like_ipv4_label(current_trimmed) && !looks_like_ipv4_label(incoming_trimmed) {
        return true;
    }
    false
}

fn is_generic_device_display_name(value: &str) -> bool {
    matches!(
        value,
        "Unclassified Device"
            | "PhotonVision Device"
            | "Limelight Device"
            | "HeliOS Device"
            | "roboRIO"
            | "Unknown device"
            | "roboRIO device"
            | "PhotonVision device"
            | "HeliOS device"
            | "Limelight device"
    )
}

fn looks_like_ipv4_label(value: &str) -> bool {
    let mut segments = 0usize;
    for segment in value.split('.') {
        if segment.is_empty() || segment.len() > 3 {
            return false;
        }
        if segment.parse::<u8>().is_err() {
            return false;
        }
        segments += 1;
    }
    segments == 4
}

fn collect_neighbor_entries() -> Vec<RawNeighborEntry> {
    let mut entries = Vec::new();

    if let Ok(ip_entries) = discover_neighbor_entries_from_ip() {
        entries.extend(ip_entries);
    }
    if let Ok(arp_entries) = discover_neighbor_entries_from_arp() {
        entries.extend(arp_entries);
    }

    entries
}

fn prime_neighbor_cache_with_likely_robot_ips() {
    let targets = likely_robot_probe_targets();
    if targets.is_empty() {
        return;
    }

    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return;
    };
    let _ = socket.set_write_timeout(Some(Duration::from_millis(150)));

    for target in targets {
        let _ = socket.send_to(&[0_u8], format!("{target}:9"));
    }

    thread::sleep(Duration::from_millis(120));
}

fn prime_neighbor_cache_for_local_frc_subnets(existing_entries: &[RawNeighborEntry]) {
    let local_addresses = discover_local_ipv4_addresses();
    let mut subnets = HashSet::new();
    let mut local_hosts: HashMap<(u8, u8, u8), HashSet<u8>> = HashMap::new();
    let mut preferred_subnets = HashSet::new();

    for entry in existing_entries {
        if is_inactive_neighbor_state(entry) || !is_probable_roborio_ip(&entry.ip) {
            continue;
        }
        let Ok(address) = entry.ip.parse::<Ipv4Addr>() else {
            continue;
        };
        let octets = address.octets();
        preferred_subnets.insert((octets[0], octets[1], octets[2]));
    }
    for ip in discover_roborio_mdns_ips() {
        let Ok(address) = ip.parse::<Ipv4Addr>() else {
            continue;
        };
        let octets = address.octets();
        preferred_subnets.insert((octets[0], octets[1], octets[2]));
    }

    for address in local_addresses {
        let octets = address.octets();
        let is_frc_like_subnet = octets[0] == 10
            || (octets[0] == 172 && octets[1] == 22 && octets[2] == 11)
            || (octets[0] == 169 && octets[1] == 254);
        if !is_frc_like_subnet {
            continue;
        }

        let key = (octets[0], octets[1], octets[2]);
        if !preferred_subnets.is_empty() && !preferred_subnets.contains(&key) {
            continue;
        }
        subnets.insert(key);
        local_hosts.entry(key).or_default().insert(octets[3]);
    }

    if subnets.is_empty() {
        return;
    }

    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return;
    };
    let _ = socket.set_write_timeout(Some(Duration::from_millis(30)));

    for subnet in subnets {
        let local_suffixes = local_hosts.get(&subnet).cloned().unwrap_or_default();
        for host in 1_u8..=254_u8 {
            if local_suffixes.contains(&host) {
                continue;
            }
            let target = format!("{}.{}.{}.{}", subnet.0, subnet.1, subnet.2, host);
            let _ = socket.send_to(&[0_u8], format!("{target}:9"));
        }
    }

    thread::sleep(Duration::from_millis(250));
}

#[cfg(test)]
mod network_neighbor_parsing_tests {
    use super::*;

    fn make_device(
        id: &str,
        status: &str,
        chips: &[&str],
        usb_location: Option<&str>,
        interface_name: Option<&str>,
    ) -> DiscoveredDevice {
        DiscoveredDevice {
            id: id.to_string(),
            display_name: id.to_string(),
            status: status.to_string(),
            connection_chips: chips.iter().map(|chip| chip.to_string()).collect(),
            ip_address: None,
            mac_address: None,
            interface_name: interface_name.map(|value| value.to_string()),
            usb_location: usb_location.map(|value| value.to_string()),
            vendor_product: None,
            runtime_product: None,
            firmware_version: None,
            os_version: None,
            telemetry_summary: None,
            detail: String::new(),
        }
    }

    #[test]
    fn normalize_usb_topology_path_handles_suffixes_and_padding() {
        assert_eq!(
            normalize_usb_topology_path("001-02.003:1.0"),
            Some("1-2.3".to_string())
        );
        assert_eq!(
            resolve_usb_topology_path_from_text("USB Path 2-4.1 · Bus 002 Device 013"),
            Some("2-4.1".to_string())
        );
    }

    #[test]
    fn dedupe_drops_bootloader_and_mounted_when_live_usb_exists_on_same_port() {
        let mut devices = vec![
            make_device(
                "bootloader",
                "bootloader",
                &["Bootloader Device"],
                Some("USB Path 1-2 · Bus 001 Device 007"),
                None,
            ),
            make_device(
                "mounted",
                "mounted",
                &["Mounted"],
                Some("USB Path 1-2 · Bus 001 Device 008"),
                None,
            ),
            make_device(
                "live",
                "online",
                &["USB IP", "HeliOS"],
                Some("USB Path 1-2 · Interface usb0"),
                Some("usb0"),
            ),
        ];

        dedupe_discovered_devices(&mut devices);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "live");
        assert_eq!(devices[0].status, "online");
    }

    #[test]
    fn dedupe_collapses_transition_states_on_same_usb_port() {
        let mut devices = vec![
            make_device(
                "bootloader-a",
                "bootloader",
                &["Bootloader Device"],
                Some("USB Path 3-1.4 · Bus 003 Device 019"),
                None,
            ),
            make_device(
                "mounted-a",
                "mounted",
                &["Mounted"],
                Some("USB Path 3-1.4 · Bus 003 Device 020"),
                None,
            ),
        ];

        dedupe_discovered_devices(&mut devices);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].status, "mounted");
    }
}
