use super::*;

#[path = "install_flow_main/ota_types_and_reader.rs"]
mod ota_types_and_reader;
pub(crate) use ota_types_and_reader::*;


#[tauri::command]
pub(crate) fn install_helios_os(
    app: tauri::AppHandle,
    request: ReleaseInstallRequest,
) -> Result<ReleaseInstallResult, String> {
    let run_id = request.run_id.clone();
    let run_id_ref = run_id.as_deref();
    let using_ota = request.prefer_ota.unwrap_or(false);
    let requested_mode = if request.mount_only.unwrap_or(false) {
        "mount"
    } else if using_ota {
        "ota"
    } else {
        "flash"
    };
    let _cancel_guard = UpdateCancelGuard::begin(run_id_ref, requested_mode);
    fail_if_update_cancelled(&app, run_id_ref, requested_mode, "resolve-image")?;
    if request.mount_only.unwrap_or(false) {
        emit_updater_progress(
            &app,
            run_id_ref,
            "mount",
            "info",
            "running",
            "Install request switched to mount-only workflow.",
        );
        return mount_helios_bootloader(
            app.clone(),
            MountRequest {
                timeout_seconds: request.timeout_seconds,
                selected_bootloader_id: request.selected_bootloader_id.clone(),
                run_id,
            },
        );
    }

    emit_updater_progress(
        &app,
        run_id_ref,
        requested_mode,
        "resolve-image",
        "running",
        "Resolving install image source.",
    );
    let (image_path, source_mode) =
        match resolve_install_image_path(&app, &request, run_id_ref, requested_mode) {
        Ok(value) => value,
        Err(error) => {
            emit_updater_progress(
                &app,
                run_id_ref,
                requested_mode,
                "resolve-image",
                "error",
                format!("Failed to resolve install image: {error}"),
            );
            return Err(error);
        }
    };
    let _ = app.emit(
        UPDATER_PROGRESS_EVENT,
        UpdaterProgressEvent {
            run_id: run_id.clone(),
            mode: requested_mode.into(),
            step: UpdaterStep::ResolveImage,
            status: UpdaterStatus::Success,
            message: "Install image resolved.".to_string(),
            timestamp_epoch_ms: epoch_ms(),
            stdout: None,
            stderr: None,
            exit_code: None,
            duration_ms: None,
            image_path: Some(image_path.clone()),
            target_path: None,
            progress_percent: None,
            bytes_written: None,
            bytes_total: None,
        },
    );
    if using_ota {
        return install_helios_os_via_ota(
            &app,
            &request,
            run_id_ref,
            image_path,
        );
    }
    let timeout_seconds = request.timeout_seconds.unwrap_or(60).clamp(15, 600);
    fail_if_update_cancelled(&app, run_id_ref, "flash", "scan-targets")?;

    emit_updater_progress(
        &app,
        run_id_ref,
        "flash",
        "scan-targets",
        "running",
        "Scanning for currently available flash targets.",
    );
    let before_targets = match discover_flash_targets() {
        Ok(targets) => targets,
        Err(error) => {
            emit_updater_progress(
                &app,
                run_id_ref,
                "flash",
                "scan-targets",
                "error",
                format!("Flash target scan failed: {error}"),
            );
            return Err(error);
        }
    };
    emit_updater_progress(
        &app,
        run_id_ref,
        "flash",
        "scan-targets",
        "success",
        format!(
            "Detected {} candidate target(s) before rpiboot.",
            before_targets.len()
        ),
    );
    let before_paths: HashSet<String> = before_targets
        .iter()
        .map(|target| target.path.clone())
        .collect();
    fail_if_update_cancelled(&app, run_id_ref, "flash", "bootloader-check")?;

    emit_updater_progress(
        &app,
        run_id_ref,
        "flash",
        "bootloader-check",
        "running",
        "Checking whether USB bootloader device is present.",
    );
    let available_bootloaders = match discover_usb_devices() {
        Ok(devices) => devices,
        Err(error) => {
            emit_updater_progress(
                &app,
                run_id_ref,
                "flash",
                "bootloader-check",
                "error",
                format!("USB bootloader check failed: {error}"),
            );
            return Err(error);
        }
    }
    .into_iter()
    .filter(|device| device.is_bootloader)
    .collect::<Vec<_>>();
    let selected_bootloader = updater_mount_helpers::resolve_selected_bootloader_device(
        &available_bootloaders,
        request.selected_bootloader_id.as_deref(),
    )?;
    let bootloader_present = selected_bootloader.is_some();
    emit_updater_progress(
        &app,
        run_id_ref,
        "flash",
        "bootloader-check",
        "success",
        if let Some(bootloader) = selected_bootloader.as_ref() {
            format!(
                "USB bootloader detected ({}:{}).",
                bootloader.vendor_id, bootloader.product_id
            )
        } else {
            "No USB bootloader detected; using existing mounted target.".to_string()
        },
    );

    let (rpiboot, selected_target_path) = if bootloader_present {
        emit_updater_progress(
            &app,
            run_id_ref,
            "flash",
            "rpiboot",
            "running",
            "Running rpiboot to expose mass-storage gadget.",
        );
        let selected_usb_path = selected_bootloader
            .as_ref()
            .and_then(|device| device.usb_path.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let rpiboot = match updater_mount_helpers::run_rpiboot_with_target(
            Some(timeout_seconds),
            selected_usb_path,
        ) {
            Ok(result) => result,
            Err(error) => {
                emit_updater_progress(
                    &app,
                    run_id_ref,
                    "flash",
                    "rpiboot",
                    "error",
                    format!("rpiboot invocation failed: {error}"),
                );
                return Err(error);
            }
        };
        fail_if_update_cancelled(&app, run_id_ref, "flash", "rpiboot")?;
        if !rpiboot.success {
            emit_operation_progress(
                &app,
                run_id_ref,
                "flash",
                "rpiboot",
                "error",
                "rpiboot failed.",
                &rpiboot,
            );
            emit_updater_progress(
                &app,
                run_id_ref,
                "flash",
                "complete",
                "error",
                "Install flow failed before flash: rpiboot did not complete.",
            );
            return Ok(ReleaseInstallResult {
                success: false,
                mode: source_mode,
                image_path: Some(image_path),
                selected_target_path: None,
                rpiboot,
                flash: None,
                message: "rpiboot failed, so flashing was not started.".to_string(),
            });
        }
        emit_operation_progress(
            &app,
            run_id_ref,
            "flash",
            "rpiboot",
            "success",
            "rpiboot completed successfully.",
            &rpiboot,
        );

        emit_updater_progress(
            &app,
            run_id_ref,
            "flash",
            "select-target",
            "running",
            "Selecting flash target after rpiboot.",
        );
        let selected_target_path = match select_flash_target_after_rpiboot(&before_paths) {
            Ok(path) => path,
            Err(error) => {
                emit_updater_progress(
                    &app,
                    run_id_ref,
                    "flash",
                    "select-target",
                    "error",
                    format!("Unable to select flash target after rpiboot: {error}"),
                );
                return Err(error);
            }
        };
        fail_if_update_cancelled(&app, run_id_ref, "flash", "select-target")?;
        let _ = app.emit(
            UPDATER_PROGRESS_EVENT,
            UpdaterProgressEvent {
                run_id: run_id.clone(),
                mode: UpdaterMode::Flash,
                step: UpdaterStep::from("select-target"),
                status: UpdaterStatus::Success,
                message: "Flash target selected.".to_string(),
                timestamp_epoch_ms: epoch_ms(),
                stdout: None,
                stderr: None,
                exit_code: None,
                duration_ms: None,
                image_path: Some(image_path.clone()),
                target_path: Some(selected_target_path.clone()),
                progress_percent: None,
                bytes_written: None,
                bytes_total: None,
            },
        );
        (rpiboot, selected_target_path)
    } else {
        emit_updater_progress(
            &app,
            run_id_ref,
            "flash",
            "rpiboot",
            "skipped",
            "Skipping rpiboot because a mounted target is already available.",
        );
        emit_updater_progress(
            &app,
            run_id_ref,
            "flash",
            "select-target",
            "running",
            "Selecting existing mounted flash target.",
        );
        let selected_target_path = match select_existing_flash_target(&before_targets) {
            Ok(path) => path,
            Err(error) => {
                emit_updater_progress(
                    &app,
                    run_id_ref,
                    "flash",
                    "select-target",
                    "error",
                    format!("Unable to select existing flash target: {error}"),
                );
                return Err(error);
            }
        };
        fail_if_update_cancelled(&app, run_id_ref, "flash", "select-target")?;
        let _ = app.emit(
            UPDATER_PROGRESS_EVENT,
            UpdaterProgressEvent {
                run_id: run_id.clone(),
                mode: UpdaterMode::Flash,
                step: UpdaterStep::from("select-target"),
                status: UpdaterStatus::Success,
                message: "Existing flash target selected.".to_string(),
                timestamp_epoch_ms: epoch_ms(),
                stdout: None,
                stderr: None,
                exit_code: None,
                duration_ms: None,
                image_path: Some(image_path.clone()),
                target_path: Some(selected_target_path.clone()),
                progress_percent: None,
                bytes_written: None,
                bytes_total: None,
            },
        );
        let rpiboot =
            skipped_rpiboot_result("Skipped rpiboot because a flash target is already available.");
        (rpiboot, selected_target_path)
    };

    emit_updater_progress(
        &app,
        run_id_ref,
        "flash",
        "flash",
        "running",
        "Writing selected image to flash target.",
    );
    fail_if_update_cancelled(&app, run_id_ref, "flash", "flash")?;
    let progress_context = FlashProgressContext {
        app: app.clone(),
        run_id: run_id.clone(),
        target_path: selected_target_path.clone(),
    };
    let flash_result = updater_mount_helpers::flash_helios_image_internal(
        &FlashRequest {
        image_path: image_path.clone(),
        device_path: selected_target_path.clone(),
        confirm_flash: true,
        },
        Some(&progress_context),
        false,
        false,
    );

    let flash = match flash_result {
        Ok(result) => result,
        Err(error) => {
            emit_updater_progress(
                &app,
                run_id_ref,
                "flash",
                "flash",
                "error",
                format!("Flash command failed before completion: {error}"),
            );
            return Err(error);
        }
    };
    let success = flash.success;
    let flash_message = flash.message.clone();
    emit_operation_progress(
        &app,
        run_id_ref,
        "flash",
        "flash",
        if success { "success" } else { "error" },
        if success {
            "Image write finished."
        } else {
            "Image write failed."
        },
        &flash,
    );
    if success {
        emit_updater_progress(
            &app,
            run_id_ref,
            "flash",
            "verify",
            "running",
            "Verifying flashed target against source image.",
        );
        fail_if_update_cancelled(&app, run_id_ref, "flash", "verify")?;
        let verify_strategy = if cfg!(target_os = "linux") {
            Some(linux_privilege_strategy()?)
        } else {
            None
        };
        #[cfg(target_os = "linux")]
        let root_partition_span = root_partition_span_from_image(Path::new(&image_path))?;
        #[cfg(target_os = "linux")]
        let had_mount_activity_before_verify = mounted_partitions_for_target_linux(&selected_target_path)
            .map(|mounted| !mounted.is_empty())
            .unwrap_or(false);
        #[cfg(target_os = "linux")]
        ensure_target_partitions_unmounted_linux(&selected_target_path)?;
        match verify_flashed_target_matches_image(
            Path::new(&image_path),
            &selected_target_path,
            verify_strategy,
            Some(&progress_context),
        ) {
            Ok(()) => {
                emit_updater_progress(
                    &app,
                    run_id_ref,
                    "flash",
                    "verify",
                    "success",
                    "Flash verification completed successfully.",
                );
            }
            Err(error) => {
                #[cfg(target_os = "linux")]
                {
                    let mismatch_offset = parse_verification_mismatch_offset(&error);
                    let mismatch_in_root_partition =
                        mismatch_offset_in_partition(mismatch_offset, root_partition_span);
                    let mount_activity_after_verify = mounted_partitions_for_target_linux(&selected_target_path)
                        .map(|mounted| !mounted.is_empty())
                        .unwrap_or(false);
                    let observed_mount_activity =
                        had_mount_activity_before_verify || mount_activity_after_verify;
                    let allow_nonfatal_root_mismatch =
                        !strict_flash_verify_enabled() && mismatch_in_root_partition;
                    if allow_nonfatal_root_mismatch {
                        let _ = ensure_target_partitions_unmounted_linux(&selected_target_path);
                        let offset_text = mismatch_offset
                            .map(|offset| offset.to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        let mount_note = if observed_mount_activity {
                            " Host auto-mount activity was observed."
                        } else {
                            " Host auto-mount activity was not directly observed, but root partition metadata drift is treated as non-fatal by default on Linux."
                        };
                        emit_updater_progress(
                            &app,
                            run_id_ref,
                            "flash",
                            "verify",
                            "skipped",
                            format!(
                                "Verification mismatch at byte offset {offset_text} inside root partition. Treating as non-fatal metadata drift.{mount_note} Set ATLAS_STRICT_FLASH_VERIFY=1 to fail hard."
                            ),
                        );
                    } else {
                        emit_updater_progress(
                            &app,
                            run_id_ref,
                            "flash",
                            "verify",
                            "error",
                            format!("Flash verification failed: {error}"),
                        );
                        return Err(format!(
                            "Flash write succeeded but verification failed: {error}"
                        ));
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    emit_updater_progress(
                        &app,
                        run_id_ref,
                        "flash",
                        "verify",
                        "error",
                        format!("Flash verification failed: {error}"),
                    );
                    return Err(format!(
                        "Flash write succeeded but verification failed: {error}"
                    ));
                }
            }
        }
        emit_updater_progress(
            &app,
            run_id_ref,
            "flash",
            "finalize-target",
            "running",
            "Finalizing flash target and attempting eject/power-cycle.",
        );
        match finalize_flashed_target(&selected_target_path) {
            Ok(message) => {
                emit_updater_progress(
                    &app,
                    run_id_ref,
                    "flash",
                    "finalize-target",
                    "success",
                    message,
                );
            }
            Err(message) => {
                emit_updater_progress(
                    &app,
                    run_id_ref,
                    "flash",
                    "finalize-target",
                    "error",
                    message.clone(),
                );
                return Err(format!(
                    "Flash write succeeded but target finalization failed: {message}"
                ));
            }
        }
    }
    emit_updater_progress(
        &app,
        run_id_ref,
        "flash",
        "complete",
        if success { "success" } else { "error" },
        if success {
            format!("Install flow completed for target {selected_target_path}.")
        } else {
            flash_message.clone()
        },
    );

    Ok(ReleaseInstallResult {
        success,
        mode: source_mode,
        image_path: Some(image_path),
        selected_target_path: Some(selected_target_path.clone()),
        rpiboot,
        flash: Some(flash),
        message: if success {
            format!("Install flow completed for target {selected_target_path}.")
        } else {
            flash_message
        },
    })
}
