fn discover_usb_devices() -> Result<Vec<UsbDevice>, String> {
    let mut devices = discover_usb_devices_with_rusb().unwrap_or_default();
    if devices.is_empty() {
        devices = discover_usb_devices_with_lsusb()?;
    }
    devices.sort_by_key(|device| Reverse(device.is_bootloader));
    Ok(devices)
}

fn discover_usb_devices_with_rusb() -> Result<Vec<UsbDevice>, String> {
    let context = rusb::Context::new()
        .map_err(|error| format!("Unable to initialize USB context: {error}"))?;
    let device_list = context
        .devices()
        .map_err(|error| format!("Unable to enumerate USB devices: {error}"))?;

    let mut devices = Vec::new();
    for device in device_list.iter() {
        let Ok(descriptor) = device.device_descriptor() else {
            continue;
        };

        let vendor_id = format!("{:04x}", descriptor.vendor_id());
        let product_id = format!("{:04x}", descriptor.product_id());

        let mut manufacturer = None;
        let mut product = None;
        if let Ok(handle) = device.open() {
            let timeout = Duration::from_millis(200);
            if let Ok(languages) = handle.read_languages(timeout) {
                if let Some(language) = languages.first() {
                    manufacturer = handle
                        .read_manufacturer_string(*language, &descriptor, timeout)
                        .ok();
                    product = handle
                        .read_product_string(*language, &descriptor, timeout)
                        .ok();
                }
            }
        }

        let description = match (manufacturer.as_deref(), product.as_deref()) {
            (Some(manufacturer), Some(product)) => format!("{manufacturer} {product}"),
            (Some(manufacturer), None) => manufacturer.to_string(),
            (None, Some(product)) => product.to_string(),
            (None, None) => format!("USB {vendor_id}:{product_id}"),
        };

        let description_lower = description.to_ascii_lowercase();
        let is_bootloader = is_rpi_bootloader_id(&vendor_id, &product_id)
            || description_lower.contains("broadcom")
            || description_lower.contains("bcm27")
            || description_lower.contains("usbboot");
        let is_helios_candidate = is_bootloader
            || description_lower.contains("raspberry")
            || description_lower.contains("helios")
            || description_lower.contains("raze");

        devices.push(UsbDevice {
            bus: format!("{:03}", device.bus_number()),
            device: format!("{:03}", device.address()),
            usb_path: usb_path_from_rusb_device(&device),
            vendor_id,
            product_id,
            description,
            is_bootloader,
            is_helios_candidate,
        });
    }

    Ok(devices)
}

fn discover_usb_devices_with_lsusb() -> Result<Vec<UsbDevice>, String> {
    let Some(lsusb_path) = find_in_path("lsusb") else {
        return Ok(Vec::new());
    };

    let output = Command::new(lsusb_path)
        .output()
        .map_err(|error| format!("Failed to execute lsusb: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "lsusb failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mut devices = Vec::new();

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 || parts[0] != "Bus" || parts[2] != "Device" || parts[4] != "ID" {
            continue;
        }

        let bus = parts[1].to_string();
        let device = parts[3].trim_end_matches(':').to_string();
        let usb_path = usb_path_from_bus_and_device(&bus, &device);
        let id = parts[5];
        let mut id_parts = id.split(':');
        let vendor_id = id_parts.next().unwrap_or_default().to_lowercase();
        let product_id = id_parts.next().unwrap_or_default().to_lowercase();
        let description = if parts.len() > 6 {
            parts[6..].join(" ")
        } else {
            "Unknown USB device".to_string()
        };

        let description_lower = description.to_lowercase();
        let is_bootloader = is_rpi_bootloader_id(&vendor_id, &product_id)
            || description_lower.contains("broadcom")
            || description_lower.contains("bcm27")
            || description_lower.contains("usbboot");
        let is_helios_candidate = is_bootloader
            || description_lower.contains("raspberry")
            || description_lower.contains("helios")
            || description_lower.contains("raze");

        devices.push(UsbDevice {
            bus,
            device,
            usb_path,
            vendor_id,
            product_id,
            description,
            is_bootloader,
            is_helios_candidate,
        });
    }

    Ok(devices)
}

fn usb_path_from_rusb_device(device: &rusb::Device<rusb::Context>) -> Option<String> {
    let ports = device.port_numbers().ok()?;
    if ports.is_empty() {
        return None;
    }
    let bus = device.bus_number().to_string();
    let tail = ports
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(".");
    Some(format!("{bus}-{tail}"))
}

fn usb_path_from_bus_and_device(bus: &str, device: &str) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let bus_value = bus.trim().parse::<u64>().ok()?;
    let device_value = device.trim().parse::<u64>().ok()?;
    let entries = fs::read_dir("/sys/bus/usb/devices").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let busnum = fs::read_to_string(path.join("busnum")).ok()?;
        let devnum = fs::read_to_string(path.join("devnum")).ok()?;
        let entry_bus = busnum.trim().parse::<u64>().ok()?;
        let entry_dev = devnum.trim().parse::<u64>().ok()?;
        if entry_bus != bus_value || entry_dev != device_value {
            continue;
        }
        let name = path.file_name()?.to_str()?.trim();
        if !name.is_empty() && name.contains('-') {
            return Some(name.to_string());
        }
    }
    None
}

fn usb_bootloader_discovery_id(usb: &UsbDevice) -> String {
    let vendor_product = format!("{}-{}", usb.vendor_id, usb.product_id);
    if let Some(usb_path) = usb
        .usb_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let sanitized = usb_path
            .chars()
            .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
            .collect::<String>();
        return format!("bootloader-{sanitized}-{vendor_product}");
    }

    format!(
        "bootloader-{}-{}-{vendor_product}",
        usb.bus.trim(),
        usb.device.trim()
    )
}

fn is_rpi_bootloader_id(vendor_id: &str, product_id: &str) -> bool {
    matches!(
        (vendor_id, product_id),
        ("0a5c", "2711") | ("0a5c", "2712") | ("0a5c", "2763") | ("0a5c", "2764")
    )
}

fn discover_flash_targets() -> Result<Vec<FlashTarget>, String> {
    if cfg!(target_os = "linux") {
        let targets = discover_flash_targets_linux()?;
        if !targets.is_empty() {
            return Ok(targets);
        }
    }

    if cfg!(target_os = "windows") {
        let targets = discover_flash_targets_windows()?;
        if !targets.is_empty() {
            return Ok(targets);
        }
    }

    if cfg!(target_os = "macos") {
        let targets = discover_flash_targets_macos()?;
        if !targets.is_empty() {
            return Ok(targets);
        }
    }

    discover_flash_targets_sysinfo()
}

fn discover_flash_targets_linux() -> Result<Vec<FlashTarget>, String> {
    let Some(lsblk_path) = find_in_path("lsblk") else {
        return Ok(Vec::new());
    };

    let output = Command::new(lsblk_path)
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,PATH,SIZE,TYPE,MODEL,TRAN,HOTPLUG,RM,MOUNTPOINTS",
        ])
        .output()
        .map_err(|error| format!("Failed to execute lsblk: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Unable to parse lsblk JSON output: {error}"))?;

    let mut targets = Vec::new();

    if let Some(block_devices) = value.get("blockdevices").and_then(Value::as_array) {
        for device in block_devices {
            let device_type = device
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if device_type != "disk" {
                continue;
            }

            let transport = device
                .get("tran")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            let hotplug = to_bool_flag(device.get("hotplug"));
            let removable = to_bool_flag(device.get("rm"));

            if !hotplug && !removable && transport.as_deref() != Some("usb") {
                continue;
            }

            let path = device
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if path.is_empty() {
                continue;
            }

            let mountpoints = collect_mountpoints(device);

            targets.push(FlashTarget {
                path,
                name: device
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                size_bytes: device
                    .get("size")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                model: device
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                transport,
                removable,
                hotplug,
                mounted: !mountpoints.is_empty(),
            });
        }
    }

    targets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(targets)
}

fn discover_flash_targets_windows() -> Result<Vec<FlashTarget>, String> {
    let Some(powershell_path) = find_in_path("powershell") else {
        return Ok(Vec::new());
    };

    let output = Command::new(powershell_path)
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_DiskDrive | Select-Object DeviceID,Model,Size,InterfaceType,MediaType | ConvertTo-Json -Compress",
        ])
        .output()
        .map_err(|error| format!("Failed to execute PowerShell disk query: {error}"))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let entries = match value {
        Value::Array(entries) => entries,
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    };

    let mut targets = Vec::new();
    for entry in entries {
        let device_id = entry
            .get("DeviceID")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if device_id.is_empty() {
            continue;
        }

        let interface = entry
            .get("InterfaceType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let media_type = entry
            .get("MediaType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();

        let removable = interface.contains("usb")
            || media_type.contains("removable")
            || media_type.contains("external");
        if !removable {
            continue;
        }

        let name = device_id
            .rsplit('\\')
            .next()
            .unwrap_or("physicaldrive")
            .to_string();
        targets.push(FlashTarget {
            path: device_id.clone(),
            name,
            size_bytes: entry
                .get("Size")
                .and_then(|value| {
                    value
                        .as_str()
                        .and_then(|text| text.parse::<u64>().ok())
                        .or_else(|| value.as_u64())
                })
                .unwrap_or_default(),
            model: entry
                .get("Model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            transport: Some("usb".to_string()),
            removable: true,
            hotplug: true,
            mounted: false,
        });
    }

    targets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(targets)
}

fn discover_flash_targets_macos() -> Result<Vec<FlashTarget>, String> {
    let Some(diskutil_path) = find_in_path("diskutil") else {
        return Ok(Vec::new());
    };

    let output = Command::new(diskutil_path)
        .arg("list")
        .output()
        .map_err(|error| format!("Failed to execute diskutil list: {error}"))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let mut targets = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("/dev/disk") {
            continue;
        }

        let lower = trimmed.to_ascii_lowercase();
        if !(lower.contains("external") || lower.contains("removable")) {
            continue;
        }

        let path = trimmed
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        if path.is_empty() {
            continue;
        }

        let name = path.rsplit('/').next().unwrap_or("disk").to_string();
        targets.push(FlashTarget {
            path,
            name,
            size_bytes: 0,
            model: "External Disk".to_string(),
            transport: Some("usb".to_string()),
            removable: true,
            hotplug: true,
            mounted: false,
        });
    }

    targets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(targets)
}
