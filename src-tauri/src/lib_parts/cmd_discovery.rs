#[path = "cmd_discovery/helpers.rs"]
mod cmd_discovery_helpers;
pub(crate) use cmd_discovery_helpers::*;

#[tauri::command]
fn discover_helios_devices(
    workspace_path: Option<String>,
) -> Result<HeliosDiscoverySnapshot, String> {
    let workspace_path = resolve_workspace_path(workspace_path);
    let workspace_exists = Path::new(&workspace_path).is_dir();

    let rpiboot_resolved = resolve_tool("rpiboot");
    let rpiboot_path = rpiboot_resolved
        .as_ref()
        .map(|tool| path_to_string(&tool.path));
    let rpiboot = ToolStatus {
        available: rpiboot_path.is_some(),
        path: rpiboot_path,
        detail: rpiboot_resolved.as_ref().map(|tool| match tool.source {
            ToolSource::Bundled => "Using bundled rpiboot binary.".to_string(),
            ToolSource::SystemPath => {
                "Using system rpiboot from PATH. Bundle rpiboot for production releases."
                    .to_string()
            }
        }),
    };

    let sudo_relevant = cfg!(target_os = "linux");
    let sudo_available = sudo_relevant && has_passwordless_sudo();
    let sudo = ToolStatus {
        available: sudo_available,
        path: if sudo_relevant {
            find_in_path("sudo").map(path_to_string)
        } else {
            None
        },
        detail: Some(if sudo_available {
            "Passwordless sudo available for privileged operations.".to_string()
        } else if !sudo_relevant {
            "Sudo checks are not required on this platform.".to_string()
        } else {
            "Passwordless sudo not available. rpiboot/flashing may fail without elevated access."
                .to_string()
        }),
    };

    let usb_devices = discover_usb_devices()?;
    let bootloader_present = usb_devices.iter().any(|device| device.is_bootloader);

    let flash_targets = discover_flash_targets()?;
    let firmware_images = if workspace_exists {
        discover_firmware_images(Path::new(&workspace_path))?
    } else {
        Vec::new()
    };
    let network_neighbors = discover_network_neighbors()?;

    let mut warnings = Vec::new();
    if !workspace_exists {
        warnings.push(format!(
            "HeliOS workspace was not found at: {workspace_path}"
        ));
    }
    if bootloader_present && !rpiboot.available {
        warnings.push(
            "A bootloader-mode device is connected, but `rpiboot` is unavailable. Install it or bundle it with the app."
                .to_string(),
        );
    }
    if sudo_relevant && !sudo_available {
        warnings.push(
            "Privileged commands are likely required for rpiboot and dd flashing. Configure sudo access for this host."
                .to_string(),
        );
    }
    if bootloader_present && flash_targets.is_empty() {
        warnings.push(
            "Bootloader device detected but no removable flash target is visible yet. Run rpiboot and rediscover."
                .to_string(),
        );
    }

    Ok(HeliosDiscoverySnapshot {
        workspace_path,
        workspace_exists,
        generated_at_epoch_ms: epoch_ms(),
        rpiboot,
        sudo,
        bootloader_present,
        usb_devices,
        flash_targets,
        firmware_images,
        network_neighbors,
        warnings,
    })
}

#[tauri::command]
async fn discover_network_devices(app: tauri::AppHandle) -> Result<DeviceDiscoverySnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || discover_network_devices_blocking(Some(app)))
        .await
        .map_err(|error| format!("Device discovery worker failed: {error}"))?
}

fn discover_network_devices_blocking(
    app: Option<tauri::AppHandle>,
) -> Result<DeviceDiscoverySnapshot, String> {
    let mut devices: Vec<DiscoveredDevice> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let usb_devices = match discover_usb_devices() {
        Ok(devices) => devices,
        Err(error) => {
            warnings.push(format!("USB discovery failed: {error}"));
            Vec::new()
        }
    };
    let flash_targets = match discover_flash_targets() {
        Ok(targets) => targets,
        Err(error) => {
            warnings.push(format!("Flash target discovery failed: {error}"));
            Vec::new()
        }
    };
    let mounted_target = select_likely_mounted_target(&flash_targets);
    let bootloader_usb_count = usb_devices
        .iter()
        .filter(|device| device.is_bootloader)
        .count();

    for usb in usb_devices.iter().filter(|device| device.is_bootloader) {
        let is_mounted = mounted_target.is_some();
        let status = if is_mounted { "mounted" } else { "bootloader" }.to_string();
        let connection_chip = if is_mounted {
            "Mounted".to_string()
        } else {
            "Bootloader Device".to_string()
        };
        let detail = if let Some(target) = mounted_target {
            let model = if target.model.trim().is_empty() {
                "Unknown model".to_string()
            } else {
                target.model.clone()
            };
            format!(
                "{} · Flash target ready at {} ({model})",
                usb.description, target.path
            )
        } else {
            usb.description.clone()
        };

        devices.push(DiscoveredDevice {
            id: usb_bootloader_discovery_id(usb),
            display_name: if is_mounted {
                "Mounted Device".to_string()
            } else {
                "Bootloader Device".to_string()
            },
            status,
            connection_chips: vec![connection_chip],
            ip_address: None,
            mac_address: None,
            interface_name: None,
            usb_location: Some(if let Some(path) = usb
                .usb_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                format!("USB Path {path} · Bus {} Device {}", usb.bus, usb.device)
            } else {
                format!("Bus {} Device {}", usb.bus, usb.device)
            }),
            vendor_product: Some(format!("{}:{}", usb.vendor_id, usb.product_id)),
            runtime_product: None,
            firmware_version: None,
            os_version: None,
            telemetry_summary: None,
            detail,
        });
    }

    if bootloader_usb_count == 0 {
        if let Some(target) = mounted_target {
            let model = if target.model.trim().is_empty() {
                "Unknown model".to_string()
            } else {
                target.model.clone()
            };

            devices.push(DiscoveredDevice {
                id: format!("mounted-{}", target.name),
                display_name: "Mounted Device".to_string(),
                status: "mounted".to_string(),
                connection_chips: vec!["Mounted".to_string()],
                ip_address: None,
                mac_address: None,
                interface_name: None,
                usb_location: Some(target.path.clone()),
                vendor_product: Some(model.clone()),
                runtime_product: None,
                firmware_version: None,
                os_version: None,
                telemetry_summary: None,
                detail: format!("Flash target ready at {} ({model})", target.path),
            });
        }
    }
    emit_discovery_progress(app.as_ref(), &devices, &warnings, true);

    let ip_candidates = match discover_helios_ip_candidates() {
        Ok(candidates) => candidates,
        Err(error) => {
            warnings.push(format!("Network candidate discovery failed: {error}"));
            Vec::new()
        }
    };
    let mut candidates_by_ip: HashMap<String, Vec<IpDeviceCandidate>> = HashMap::new();
    for candidate in ip_candidates {
        candidates_by_ip
            .entry(candidate.ip.clone())
            .or_default()
            .push(candidate);
    }
    let (runtime_tx, runtime_rx) =
        std::sync::mpsc::channel::<(String, Option<HeliosRuntimeInfo>)>();
    for ip in candidates_by_ip.keys().cloned().collect::<Vec<_>>() {
        let worker_tx = runtime_tx.clone();
        thread::spawn(move || {
            let runtime_info = if is_probable_roborio_ip(&ip) {
                fetch_roborio_runtime_info(&ip).or_else(|| fetch_helios_runtime_info(&ip))
            } else {
                fetch_helios_runtime_info(&ip).or_else(|| {
                    if is_probable_roborio_ip(&ip) {
                        fetch_roborio_runtime_info(&ip)
                    } else {
                        None
                    }
                })
            };
            let _ = worker_tx.send((ip, runtime_info));
        });
    }
    drop(runtime_tx);

    let mut live_host_cache: HashMap<String, bool> = HashMap::new();
    let mut is_ip_live = |ip: &str| -> bool {
        if let Some(cached) = live_host_cache.get(ip) {
            return *cached;
        }
        let live = is_host_runtime_responsive(ip);
        live_host_cache.insert(ip.to_string(), live);
        live
    };

    let runtime_probe_deadline = Instant::now() + Duration::from_millis(2_800);
    while !candidates_by_ip.is_empty() {
        let now = Instant::now();
        if now >= runtime_probe_deadline {
            break;
        }
        let wait_time = runtime_probe_deadline
            .saturating_duration_since(now)
            .min(Duration::from_millis(220));
        match runtime_rx.recv_timeout(wait_time) {
            Ok((ip, runtime_info)) => {
                let Some(candidates) = candidates_by_ip.remove(&ip) else {
                    continue;
                };
                let is_roborio_target_ip = is_probable_roborio_ip(&ip);
                let include_without_runtime = if is_roborio_target_ip {
                    is_roborio_candidate_reachable(&ip)
                } else {
                    is_ip_live(&ip)
                };
                let has_roborio_candidate_signal = candidates.iter().any(is_likely_roborio_ip_candidate);
                let runtime_has_identity = runtime_info
                    .as_ref()
                    .and_then(|info| normalize_nonempty_text(info.runtime_product.clone()))
                    .is_some();
                for candidate in candidates {
                    let has_candidate_signal = has_network_candidate_signal(&candidate);
                    if is_roborio_target_ip
                        && !runtime_has_identity
                        && !include_without_runtime
                        && !has_roborio_candidate_signal
                    {
                        // Avoid surfacing transient "Unknown device" rows for roboRIO-class IPs.
                        // roboRIO is added in the dedicated roboRIO pass below.
                        continue;
                    }
                    if runtime_info.is_none() && !include_without_runtime && !has_candidate_signal
                    {
                        continue;
                    }
                    devices.push(build_discovered_ip_device(candidate, runtime_info.clone()));
                }
                emit_discovery_progress(app.as_ref(), &devices, &warnings, true);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    for candidates in candidates_by_ip.into_values() {
        for candidate in candidates {
            if is_probable_roborio_ip(&candidate.ip) {
                // Defer unresolved roboRIO-class IPs to dedicated roboRIO classification below.
                continue;
            }
            if !is_ip_live(&candidate.ip) && !has_network_candidate_signal(&candidate) {
                continue;
            }
            devices.push(build_discovered_ip_device(candidate, None));
        }
    }
    emit_discovery_progress(app.as_ref(), &devices, &warnings, true);

    let neighbors = discover_network_neighbors().unwrap_or_default();
    let active_neighbors = neighbors
        .iter()
        .filter(|neighbor| is_active_connection_neighbor_state(neighbor.state.as_deref()))
        .cloned()
        .collect::<Vec<_>>();
    let roborio_neighbor = find_roborio_neighbor(&active_neighbors).or_else(|| find_roborio_neighbor(&neighbors));
    if let Some(roborio) = roborio_neighbor.filter(is_live_roborio_neighbor_candidate) {
        let interface = roborio
            .interface
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let connected_over_usb = is_roborio_usb_link(&roborio.ip, Some(&interface));
        let state_text = roborio
            .state
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let mut chips = vec![
            if connected_over_usb {
                "USB IP".to_string()
            } else {
                "Network IP".to_string()
            },
            "roboRIO".to_string(),
        ];
        chips.sort();
        chips.dedup();

        if let Some(existing) = devices.iter_mut().find(|device| {
            discovered_device_matches_identity(device, &roborio.ip, roborio.mac.as_deref())
        }) {
            existing.status = "online".to_string();
            for chip in chips {
                if !existing.connection_chips.contains(&chip) {
                    existing.connection_chips.push(chip);
                }
            }
            existing.connection_chips.sort();
            existing.connection_chips.dedup();
            if existing.ip_address.is_none() {
                existing.ip_address = Some(roborio.ip.clone());
            }
            if existing.mac_address.is_none() {
                existing.mac_address = roborio.mac.clone();
            }
            if existing.interface_name.is_none() {
                existing.interface_name = Some(interface.clone());
            }
            if connected_over_usb {
                existing.usb_location = Some(format!("Interface {interface}"));
            }
            existing.runtime_product = Some("roboRIO".to_string());
            let current_name = existing.display_name.trim().to_ascii_lowercase();
            if current_name.is_empty()
                || current_name == "unknown device"
                || current_name == "unclassified device"
                || current_name == "network device"
                || should_prefer_display_name(&existing.display_name, "roboRIO device")
            {
                existing.display_name = "roboRIO device".to_string();
            }
            existing.detail = format!(
                "roboRIO candidate at {} on {} · State: {state_text}",
                roborio.ip, interface
            );
        } else {
            devices.push(DiscoveredDevice {
                id: format!("roborio-{}", roborio.ip.replace('.', "-")),
                display_name: "roboRIO device".to_string(),
                status: "online".to_string(),
                connection_chips: chips,
                ip_address: Some(roborio.ip.clone()),
                mac_address: roborio.mac.clone(),
                interface_name: Some(interface.clone()),
                usb_location: if connected_over_usb {
                    Some(format!("Interface {interface}"))
                } else {
                    None
                },
                vendor_product: None,
                runtime_product: Some("roboRIO".to_string()),
                firmware_version: None,
                os_version: None,
                telemetry_summary: None,
                detail: format!(
                    "roboRIO candidate at {} on {} · State: {state_text}",
                    roborio.ip, interface
                ),
            });
        }
    }
    emit_discovery_progress(app.as_ref(), &devices, &warnings, true);

    devices.sort_by(|left, right| {
        discovery_rank(&left.status, &left.connection_chips)
            .cmp(&discovery_rank(&right.status, &right.connection_chips))
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.ip_address.cmp(&right.ip_address))
    });

    dedupe_discovered_devices(&mut devices);

    if devices.is_empty() {
        warnings.push(
            "No device candidates found. Connect roboRIO, PhotonVision, Limelight, or HeliOS over USB/network and retry discovery."
                .to_string(),
        );
    }
    emit_discovery_progress(app.as_ref(), &devices, &warnings, false);

    Ok(DeviceDiscoverySnapshot {
        generated_at_epoch_ms: epoch_ms(),
        devices,
        warnings,
    })
}
