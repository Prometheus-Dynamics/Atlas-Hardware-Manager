#[path = "updater_mount_rpiboot_flash/helpers.rs"]
pub(crate) mod updater_mount_helpers;

pub(crate) fn mount_helios_bootloader(
    app: tauri::AppHandle,
    request: MountRequest,
) -> Result<ReleaseInstallResult, String> {
    let run_id = request.run_id.clone();
    let run_id_ref = run_id.as_deref();
    let _cancel_guard = UpdateCancelGuard::begin(run_id_ref, "mount");
    let timeout_seconds = request.timeout_seconds.unwrap_or(60).clamp(15, 600);
    fail_if_update_cancelled(&app, run_id_ref, "mount", "scan-targets")?;
    emit_updater_progress(
        &app,
        run_id_ref,
        "mount",
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
                "mount",
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
        "mount",
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
    fail_if_update_cancelled(&app, run_id_ref, "mount", "bootloader-check")?;

    emit_updater_progress(
        &app,
        run_id_ref,
        "mount",
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
                "mount",
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
        "mount",
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
            "mount",
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
                    "mount",
                    "rpiboot",
                    "error",
                    format!("rpiboot invocation failed: {error}"),
                );
                return Err(error);
            }
        };
        fail_if_update_cancelled(&app, run_id_ref, "mount", "rpiboot")?;
        if !rpiboot.success {
            emit_operation_progress(
                &app,
                run_id_ref,
                "mount",
                "rpiboot",
                "error",
                "rpiboot failed.",
                &rpiboot,
            );
            emit_updater_progress(
                &app,
                run_id_ref,
                "mount",
                "complete",
                "error",
                "Mount flow failed because rpiboot did not complete.",
            );
            return Ok(ReleaseInstallResult {
                success: false,
                mode: "mount".to_string(),
                image_path: None,
                selected_target_path: None,
                rpiboot,
                flash: None,
                message: "rpiboot failed, so mounting did not complete.".to_string(),
            });
        }
        emit_operation_progress(
            &app,
            run_id_ref,
            "mount",
            "rpiboot",
            "success",
            "rpiboot completed successfully.",
            &rpiboot,
        );

        emit_updater_progress(
            &app,
            run_id_ref,
            "mount",
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
                    "mount",
                    "select-target",
                    "error",
                    format!("Unable to select flash target after rpiboot: {error}"),
                );
                return Err(error);
            }
        };
        fail_if_update_cancelled(&app, run_id_ref, "mount", "select-target")?;
        let _ = app.emit(
            UPDATER_PROGRESS_EVENT,
            UpdaterProgressEvent {
                run_id: run_id.clone(),
                mode: UpdaterMode::Mount,
                step: UpdaterStep::from("select-target"),
                status: UpdaterStatus::Success,
                message: "Flash target selected.".to_string(),
                timestamp_epoch_ms: epoch_ms(),
                stdout: None,
                stderr: None,
                exit_code: None,
                duration_ms: None,
                image_path: None,
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
            "mount",
            "rpiboot",
            "skipped",
            "Skipping rpiboot because a mounted target is already available.",
        );
        emit_updater_progress(
            &app,
            run_id_ref,
            "mount",
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
                    "mount",
                    "select-target",
                    "error",
                    format!("Unable to select existing flash target: {error}"),
                );
                return Err(error);
            }
        };
        fail_if_update_cancelled(&app, run_id_ref, "mount", "select-target")?;
        let _ = app.emit(
            UPDATER_PROGRESS_EVENT,
            UpdaterProgressEvent {
                run_id: run_id.clone(),
                mode: UpdaterMode::Mount,
                step: UpdaterStep::from("select-target"),
                status: UpdaterStatus::Success,
                message: "Existing flash target selected.".to_string(),
                timestamp_epoch_ms: epoch_ms(),
                stdout: None,
                stderr: None,
                exit_code: None,
                duration_ms: None,
                image_path: None,
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
        "mount",
        "complete",
        "success",
        format!("Mount completed. Flash target available at {selected_target_path}."),
    );
    Ok(ReleaseInstallResult {
        success: true,
        mode: "mount".to_string(),
        image_path: None,
        selected_target_path: Some(selected_target_path.clone()),
        rpiboot,
        flash: None,
        message: format!("Mount completed. Flash target available at {selected_target_path}."),
    })
}
