use super::*;

#[tauri::command]
pub(crate) fn run_rpiboot(timeout_seconds: Option<u64>) -> Result<OperationResult, String> {
    run_rpiboot_with_target(timeout_seconds, None)
}

pub(crate) fn run_rpiboot_with_target(
    timeout_seconds: Option<u64>,
    usb_path: Option<&str>,
) -> Result<OperationResult, String> {
    if cfg!(target_os = "windows")
        && !is_windows_elevated().unwrap_or(false) {
            let prompt = request_elevation_prompt("rpiboot")?;
            return Err(format!(
                "rpiboot on Windows requires Administrator privileges. {prompt}"
            ));
        }
    if cfg!(target_os = "macos") && !is_unix_root() {
        let prompt = request_elevation_prompt("rpiboot")?;
        return Err(format!(
            "rpiboot on macOS requires root privileges. {prompt}"
        ));
    }

    let rpiboot_tool = resolve_required_tool_with_source("rpiboot")
        .map_err(|_| "rpiboot was not found. Install it or bundle it with the app.".to_string())?;
    if !cfg!(debug_assertions) && rpiboot_tool.source != ToolSource::Bundled {
        return Err(
            "rpiboot must be bundled with the production app for reliable cross-platform deploys."
                .to_string(),
        );
    }
    let rpiboot_path_string = path_to_string(&rpiboot_tool.path);
    let rpiboot_boot_dir = resolve_rpiboot_boot_dir(&rpiboot_tool.path);
    if rpiboot_boot_dir.is_none() && rpiboot_tool.source == ToolSource::Bundled {
        return Err(
            "Bundled rpiboot is missing `mass-storage-gadget64`. Rebuild bundled rpiboot resources."
                .to_string(),
        );
    }
    let mut rpiboot_args = Vec::new();
    if let Some(boot_dir) = rpiboot_boot_dir {
        rpiboot_args.push("-d".to_string());
        rpiboot_args.push(path_to_string(boot_dir));
    }
    if let Some(path) = usb_path.map(str::trim).filter(|value| !value.is_empty()) {
        rpiboot_args.push("-p".to_string());
        rpiboot_args.push(path.to_string());
    }

    let timeout_seconds = timeout_seconds.unwrap_or(45).clamp(10, 600);

    let result = if cfg!(target_os = "linux") {
        match linux_privilege_strategy()? {
            LinuxPrivilegeStrategy::Direct => {
                let rpiboot_arg_refs = rpiboot_args.iter().map(String::as_str).collect::<Vec<_>>();
                run_command_with_timeout(
                    &rpiboot_path_string,
                    &rpiboot_arg_refs,
                    Duration::from_secs(timeout_seconds),
                )?
            }
            LinuxPrivilegeStrategy::SudoNoPrompt => {
                let mut sudo_args = vec!["-n".to_string(), rpiboot_path_string.clone()];
                sudo_args.extend(rpiboot_args.clone());
                let sudo_arg_refs = sudo_args.iter().map(String::as_str).collect::<Vec<_>>();
                run_command_with_timeout("sudo", &sudo_arg_refs, Duration::from_secs(timeout_seconds))?
            }
            LinuxPrivilegeStrategy::PkexecPrompt => {
                let wrapped_args =
                    build_pkexec_shell_wrapper_args(&rpiboot_path_string, &rpiboot_args)?;
                let arg_refs = wrapped_args.iter().map(String::as_str).collect::<Vec<_>>();
                run_command_with_timeout("pkexec", &arg_refs, Duration::from_secs(timeout_seconds))?
            }
            LinuxPrivilegeStrategy::SudoPrompt => {
                let askpass_path = resolve_sudo_askpass_helper();
                let mut sudo_args = Vec::new();
                if askpass_path.is_some() {
                    sudo_args.push("-A".to_string());
                }
                sudo_args.push(rpiboot_path_string.clone());
                sudo_args.extend(rpiboot_args.clone());
                let sudo_arg_refs = sudo_args.iter().map(String::as_str).collect::<Vec<_>>();
                if let Some(path) = askpass_path.as_ref() {
                    let askpass_path_string = path_to_string(path);
                    run_command_with_timeout_and_env(
                        "sudo",
                        &sudo_arg_refs,
                        &[("SUDO_ASKPASS", askpass_path_string.as_str())],
                        Duration::from_secs(timeout_seconds),
                    )?
                } else {
                    run_command_with_timeout("sudo", &sudo_arg_refs, Duration::from_secs(timeout_seconds))?
                }
            }
        }
    } else {
        let rpiboot_arg_refs = rpiboot_args.iter().map(String::as_str).collect::<Vec<_>>();
        run_command_with_timeout(
            &rpiboot_path_string,
            &rpiboot_arg_refs,
            Duration::from_secs(timeout_seconds),
        )?
    };

    let stdout = trim_output(&result.output.stdout);
    let stderr = trim_output(&result.output.stderr);

    let message = if result.canceled {
        "rpiboot canceled by user request.".to_string()
    } else if result.timed_out {
        format!(
            "rpiboot timed out after {timeout_seconds}s. Keep the device in USB boot mode and retry."
        )
    } else if result.output.status.success() {
        "rpiboot completed. Re-run discovery to verify a flash target is now visible.".to_string()
    } else {
        "rpiboot failed. Review stderr for host permission or USB connection issues.".to_string()
    };

    Ok(OperationResult {
        success: result.output.status.success() && !result.timed_out && !result.canceled,
        exit_code: result.output.status.code(),
        duration_ms: result.duration_ms,
        stdout,
        stderr,
        message,
        timed_out: result.timed_out,
    })
}

pub(crate) fn resolve_selected_bootloader_device(
    bootloaders: &[UsbDevice],
    selected_bootloader_id: Option<&str>,
) -> Result<Option<UsbDevice>, String> {
    if bootloaders.is_empty() {
        return Ok(None);
    }

    let selected_id = selected_bootloader_id
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(selected_id) = selected_id {
        let selected = bootloaders
            .iter()
            .find(|device| {
                usb_bootloader_discovery_id(device) == selected_id
                    || legacy_usb_bootloader_discovery_id(device) == selected_id
            })
            .cloned()
            .ok_or_else(|| {
                "Selected bootloader device is no longer connected. Re-run discovery and retry."
                    .to_string()
            })?;

        if bootloaders.len() > 1
            && selected
                .usb_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
        {
            return Err(
                "Multiple bootloader devices are connected, but the selected device has no USB path for targeted rpiboot. Reconnect the selected device on a direct USB port and retry."
                    .to_string(),
            );
        }
        return Ok(Some(selected));
    }

    if bootloaders.len() == 1 {
        return Ok(bootloaders.first().cloned());
    }

    Err(
        "Multiple bootloader devices detected. Select the specific bootloader device in the device list and retry."
            .to_string(),
    )
}

pub(crate) fn legacy_usb_bootloader_discovery_id(usb: &UsbDevice) -> String {
    format!(
        "bootloader-{}-{}-{}-{}",
        usb.bus, usb.device, usb.vendor_id, usb.product_id
    )
}

#[tauri::command]
pub(crate) fn flash_helios_image(request: FlashRequest) -> Result<OperationResult, String> {
    flash_helios_image_internal(&request, None, true, true)
}

pub(crate) fn flash_helios_image_internal(
    request: &FlashRequest,
    progress_context: Option<&FlashProgressContext>,
    verify_after_write: bool,
    enforce_target_signature: bool,
) -> Result<OperationResult, String> {
    if is_update_cancel_requested() {
        return Err("Update canceled by user.".to_string());
    }

    if !request.confirm_flash {
        return Err("Flash confirmation is required before writing to a block device.".to_string());
    }

    if cfg!(target_os = "linux") {
        let _ = linux_privilege_strategy()?;
    } else if cfg!(target_os = "windows") {
        if !is_windows_elevated().unwrap_or(false) {
            let prompt = request_elevation_prompt("flashing")?;
            return Err(format!(
                "Raw disk flashing requires Administrator privileges on Windows. {prompt}"
            ));
        }
    } else if cfg!(target_os = "macos") {
        if !is_unix_root() {
            let prompt = request_elevation_prompt("flashing")?;
            return Err(format!(
                "Raw disk flashing requires root privileges on macOS. {prompt}"
            ));
        }
    } else if !is_process_elevated() {
        return Err(
            "Raw disk flashing requires elevated privileges on this OS. Relaunch Atlas Hardware Manager with administrator/root rights."
                .to_string(),
        );
    }

    if request.device_path.trim().is_empty()
        || !is_supported_flash_target_path(&request.device_path)
    {
        return Err(
            "Target path must be a raw device path (Linux/macOS: /dev/*, Windows: \\\\.\\PhysicalDriveN)."
                .to_string(),
        );
    }

    let image_path = Path::new(&request.image_path);
    if !image_path.is_file() {
        return Err(format!(
            "Image file does not exist or is not a file: {}",
            request.image_path
        ));
    }

    let allowed_targets = discover_flash_targets()?;
    if !allowed_targets
        .iter()
        .any(|target| target.path == request.device_path)
    {
        return Err(format!(
            "Selected target '{}' is not in the removable/USB flash target list.",
            request.device_path
        ));
    }

    if enforce_target_signature {
        #[cfg(target_os = "linux")]
        {
            let selected_target = allowed_targets
                .iter()
                .find(|target| target.path == request.device_path)
                .ok_or_else(|| {
                    format!(
                        "Selected target '{}' is no longer present in flash target discovery.",
                        request.device_path
                    )
                })?;
            if !looks_like_helios_flash_target(selected_target) {
                return Err(format!(
                    "Selected target '{}' does not match the expected HeliOS/Raspberry Pi signature. Refusing to flash to avoid writing the wrong disk.",
                    request.device_path
                ));
            }
        }
    }

    #[cfg(target_os = "linux")]
    ensure_target_partitions_unmounted_linux(&request.device_path)?;

    let lower_name = image_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();

    let started = Instant::now();
    let linux_strategy = if cfg!(target_os = "linux") {
        Some(linux_privilege_strategy()?)
    } else {
        None
    };

    let output = if cfg!(target_os = "linux") {
        let strategy = linux_strategy.unwrap_or(LinuxPrivilegeStrategy::Direct);
        if lower_name.ends_with(".img.xz") || lower_name.ends_with(".wic.xz") {
            flash_compressed_xz(image_path, &request.device_path, strategy, progress_context)?
        } else {
            flash_raw_image(image_path, &request.device_path, strategy, progress_context)?
        }
    } else {
        flash_with_rust_stream(image_path, &request.device_path, progress_context)?
    };

    if verify_after_write && output.status.success() {
        let skip_interactive_verify = cfg!(target_os = "linux")
            && matches!(
                linux_strategy,
                Some(LinuxPrivilegeStrategy::PkexecPrompt | LinuxPrivilegeStrategy::SudoPrompt)
            );
        if skip_interactive_verify {
            if let Some(context) = progress_context {
                emit_updater_progress(
                    &context.app,
                    context.run_id.as_deref(),
                    "flash",
                    "verify",
                    "skipped",
                    "Skipped post-write verification to avoid an additional elevation prompt.",
                );
            }
        } else {
            let root_partition_span = root_partition_span_from_image(image_path)?;
            #[cfg(target_os = "linux")]
            let had_mount_activity_before_verify = mounted_partitions_for_target_linux(&request.device_path)
                .map(|mounted| !mounted.is_empty())
                .unwrap_or(false);
            #[cfg(target_os = "linux")]
            {
                // Desktop automounters can remount freshly flashed partitions and mutate
                // filesystem metadata before verification. Force unmount again right before
                // verification so byte-compare reads stable raw media.
                ensure_target_partitions_unmounted_linux(&request.device_path)?;
            }
            if let Some(context) = progress_context {
                emit_updater_progress(
                    &context.app,
                    context.run_id.as_deref(),
                    "flash",
                    "verify",
                    "running",
                    "Verifying flashed target against source image.",
                );
            }
            let verify_result = verify_flashed_target_matches_image(
                image_path,
                &request.device_path,
                linux_strategy,
                progress_context,
            );
            match verify_result {
                Ok(()) => {
                    if let Some(context) = progress_context {
                        emit_updater_progress(
                            &context.app,
                            context.run_id.as_deref(),
                            "flash",
                            "verify",
                            "success",
                            "Flash verification completed successfully.",
                        );
                    }
                }
                Err(error) => {
                    #[cfg(target_os = "linux")]
                    {
                        let mismatch_offset = parse_verification_mismatch_offset(&error);
                        let mount_activity_after_verify = mounted_partitions_for_target_linux(&request.device_path)
                            .map(|mounted| !mounted.is_empty())
                            .unwrap_or(false);
                        let mismatch_in_root_partition =
                            mismatch_offset_in_partition(mismatch_offset, root_partition_span);
                        let observed_mount_activity =
                            had_mount_activity_before_verify || mount_activity_after_verify;
                        let allow_nonfatal_root_mismatch = !strict_flash_verify_enabled()
                            && mismatch_in_root_partition;
                        if allow_nonfatal_root_mismatch {
                            let _ = ensure_target_partitions_unmounted_linux(&request.device_path);
                            if let Some(context) = progress_context {
                                let offset_text = mismatch_offset
                                    .map(|offset| offset.to_string())
                                    .unwrap_or_else(|| "unknown".to_string());
                                let mount_note = if observed_mount_activity {
                                    " Host auto-mount activity was observed."
                                } else {
                                    " Host auto-mount activity was not directly observed, but root partition metadata drift is treated as non-fatal by default on Linux."
                                };
                                emit_updater_progress(
                                    &context.app,
                                    context.run_id.as_deref(),
                                    "flash",
                                    "verify",
                                    "skipped",
                                    format!(
                                        "Verification mismatch at byte offset {offset_text} inside root partition. Treating as non-fatal metadata drift.{mount_note} Set ATLAS_STRICT_FLASH_VERIFY=1 to fail hard."
                                    ),
                                );
                            }
                        } else {
                            return Err(error);
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        return Err(error);
                    }
                }
            }
        }
    }

    let duration_ms = started.elapsed().as_millis() as u64;
    let stdout = trim_output(&output.stdout);
    let stderr = trim_output(&output.stderr);

    Ok(OperationResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        duration_ms,
        stdout,
        stderr: stderr.clone(),
        message: if output.status.success() {
            format!(
                "Flashing completed successfully for {}.",
                request.device_path
            )
        } else if stderr.to_ascii_lowercase().contains("canceled by user")
            || stderr.to_ascii_lowercase().contains("cancelled by user")
        {
            format!("Flashing canceled for {}.", request.device_path)
        } else {
            format!(
                "Flashing failed for {}. Check stderr for dd/xz errors.",
                request.device_path
            )
        },
        timed_out: false,
    })
}
