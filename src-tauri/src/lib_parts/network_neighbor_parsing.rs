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

fn likely_robot_probe_targets() -> Vec<String> {
    let local_addresses = discover_local_ipv4_addresses();
    let local_ip_strings = local_addresses
        .iter()
        .map(ToString::to_string)
        .collect::<HashSet<_>>();

    let mut targets = HashSet::new();
    let common_host_suffixes = [2_u8, 11_u8, 12_u8, 13_u8, 14_u8];
    for suffix in common_host_suffixes {
        targets.insert(format!("172.22.11.{suffix}"));
    }

    for address in local_addresses {
        let octets = address.octets();
        if octets[0] == 10 {
            for suffix in common_host_suffixes {
                targets.insert(format!("10.{}.{}.{suffix}", octets[1], octets[2]));
            }
            continue;
        }
        if octets[0] == 172 && octets[1] == 22 && octets[2] == 11 {
            for suffix in common_host_suffixes {
                targets.insert(format!("172.22.11.{suffix}"));
            }
            continue;
        }
        if octets[0] == 192 && octets[1] == 168 {
            for suffix in common_host_suffixes {
                targets.insert(format!("192.168.{}.{suffix}", octets[2]));
            }
            continue;
        }
        if octets[0] == 169 && octets[1] == 254 {
            for suffix in common_host_suffixes {
                targets.insert(format!("169.254.{}.{suffix}", octets[2]));
            }
        }
    }

    let mut values = targets
        .into_iter()
        .filter(|ip| !local_ip_strings.contains(ip))
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn discover_local_ipv4_addresses() -> Vec<Ipv4Addr> {
    let mut unique = HashSet::new();
    let mut addresses = Vec::new();

    for address in discover_local_ipv4_addresses_from_ip() {
        if address.is_loopback() {
            continue;
        }
        if unique.insert(address) {
            addresses.push(address);
        }
    }
    for address in discover_local_ipv4_addresses_from_ifconfig() {
        if address.is_loopback() {
            continue;
        }
        if unique.insert(address) {
            addresses.push(address);
        }
    }
    for address in discover_local_ipv4_addresses_from_ipconfig() {
        if address.is_loopback() {
            continue;
        }
        if unique.insert(address) {
            addresses.push(address);
        }
    }

    addresses
}

fn discover_local_ipv4_addresses_from_ip() -> Vec<Ipv4Addr> {
    let Some(ip_path) = find_in_path("ip") else {
        return Vec::new();
    };

    let Ok(output) = Command::new(ip_path)
        .args(["-j", "-4", "addr", "show", "up"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Vec::new();
    };

    let mut addresses = Vec::new();
    let Some(interfaces) = value.as_array() else {
        return addresses;
    };

    for interface in interfaces {
        let ifname = interface
            .get("ifname")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !ifname.is_empty() && should_ignore_interface(ifname) {
            continue;
        }

        let Some(addr_info) = interface.get("addr_info").and_then(Value::as_array) else {
            continue;
        };
        for entry in addr_info {
            if entry.get("family").and_then(Value::as_str) != Some("inet") {
                continue;
            }
            let Some(local) = entry.get("local").and_then(Value::as_str) else {
                continue;
            };
            let Ok(address) = local.parse::<Ipv4Addr>() else {
                continue;
            };
            addresses.push(address);
        }
    }

    addresses
}

fn discover_local_ipv4_addresses_from_ifconfig() -> Vec<Ipv4Addr> {
    let Some(ifconfig_path) = find_in_path("ifconfig") else {
        return Vec::new();
    };

    let Ok(output) = Command::new(ifconfig_path).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut addresses = Vec::new();
    let mut current_interface = None::<String>;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let is_header = !line.starts_with(' ') && !line.starts_with('\t');
        if is_header {
            let name = line
                .split(':')
                .next()
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            current_interface = if name.is_empty() { None } else { Some(name) };
            continue;
        }

        let Some(interface_name) = current_interface.as_deref() else {
            continue;
        };
        if should_ignore_interface(interface_name) {
            continue;
        }

        let trimmed = line.trim();
        if !trimmed.starts_with("inet ") {
            continue;
        }

        let ip_token = trimmed.split_whitespace().nth(1).unwrap_or_default();
        if let Ok(address) = ip_token.parse::<Ipv4Addr>() {
            addresses.push(address);
        }
    }

    addresses
}

fn discover_local_ipv4_addresses_from_ipconfig() -> Vec<Ipv4Addr> {
    let Some(ipconfig_path) = find_in_path("ipconfig") else {
        return Vec::new();
    };

    let Ok(output) = Command::new(ipconfig_path).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut addresses = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let lowered = line.to_ascii_lowercase();
        if !lowered.contains("ipv4") {
            continue;
        }
        let Some(ip_text) = extract_first_ipv4(line) else {
            continue;
        };
        let Ok(address) = ip_text.parse::<Ipv4Addr>() else {
            continue;
        };
        addresses.push(address);
    }

    addresses
}
