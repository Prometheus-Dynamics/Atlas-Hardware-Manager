use super::*;

pub(crate) fn likely_robot_probe_targets() -> Vec<String> {
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

pub(crate) fn discover_local_ipv4_addresses() -> Vec<Ipv4Addr> {
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

pub(crate) fn discover_local_ipv4_addresses_from_ip() -> Vec<Ipv4Addr> {
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

pub(crate) fn discover_local_ipv4_addresses_from_ifconfig() -> Vec<Ipv4Addr> {
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

pub(crate) fn discover_local_ipv4_addresses_from_ipconfig() -> Vec<Ipv4Addr> {
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
