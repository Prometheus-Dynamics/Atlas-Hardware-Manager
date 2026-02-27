#[tauri::command]
fn install_helios_os(
    app: tauri::AppHandle,
    request: ReleaseInstallRequest,
) -> Result<ReleaseInstallResult, String> {
    let _cancel_guard = UpdateCancelGuard::begin();
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
    let (image_path, source_mode) = match resolve_install_image_path(&request) {
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
            mode: requested_mode.to_string(),
            step: "resolve-image".to_string(),
            status: "success".to_string(),
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
    let selected_bootloader = resolve_selected_bootloader_device(
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
        let rpiboot = match run_rpiboot_with_target(
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
                mode: "flash".to_string(),
                step: "select-target".to_string(),
                status: "success".to_string(),
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
                mode: "flash".to_string(),
                step: "select-target".to_string(),
                status: "success".to_string(),
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
    let flash_result = flash_helios_image_internal(
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

#[derive(Debug, Deserialize)]
struct OtaUploadResponse {
    image_url: String,
    filename: Option<String>,
    size_bytes: Option<u64>,
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OtaApplyResponse {
    update_id: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct OtaRuntimeState {
    update_id: Option<String>,
    stage: Option<String>,
    progress_percent: Option<f64>,
    last_error: Option<String>,
}

struct OtaUploadProgressReader {
    inner: fs::File,
    app: tauri::AppHandle,
    run_id: Option<String>,
    endpoint_label: String,
    bytes_total: Option<u64>,
    bytes_read: u64,
    last_percent_bucket: i32,
    last_emitted_bytes: u64,
}

impl OtaUploadProgressReader {
    fn new(
        inner: fs::File,
        app: tauri::AppHandle,
        run_id: Option<String>,
        endpoint_label: String,
        bytes_total: Option<u64>,
    ) -> Self {
        Self {
            inner,
            app,
            run_id,
            endpoint_label,
            bytes_total,
            bytes_read: 0,
            last_percent_bucket: -1,
            last_emitted_bytes: 0,
        }
    }

    fn emit_progress(&mut self, force: bool) {
        if let Some(total) = self.bytes_total.filter(|value| *value > 0) {
            let percent = ((self.bytes_read as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
            let bucket = (percent / 2.0).floor() as i32;
            if !force && bucket <= self.last_percent_bucket {
                return;
            }
            self.last_percent_bucket = bucket;
            emit_ota_step_progress_with_bytes(
                &self.app,
                self.run_id.as_deref(),
                "ota-upload",
                "running",
                format!(
                    "Uploading OTA image to {}: {:.1}% ({}/{} bytes).",
                    self.endpoint_label, percent, self.bytes_read, total
                ),
                Some(percent),
                Some(self.bytes_read),
                Some(total),
            );
            return;
        }

        if !force && self.bytes_read.saturating_sub(self.last_emitted_bytes) < 8 * 1024 * 1024 {
            return;
        }
        self.last_emitted_bytes = self.bytes_read;
        emit_ota_step_progress_with_bytes(
            &self.app,
            self.run_id.as_deref(),
            "ota-upload",
            "running",
            format!(
                "Uploading OTA image to {}: {} bytes sent.",
                self.endpoint_label, self.bytes_read
            ),
            None,
            Some(self.bytes_read),
            None,
        );
    }
}

impl Read for OtaUploadProgressReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes = self.inner.read(buf)?;
        if bytes > 0 {
            self.bytes_read = self.bytes_read.saturating_add(bytes as u64);
            self.emit_progress(false);
        } else {
            self.emit_progress(true);
        }
        Ok(bytes)
    }
}

fn install_helios_os_via_ota(
    app: &tauri::AppHandle,
    request: &ReleaseInstallRequest,
    run_id_ref: Option<&str>,
    image_path: String,
) -> Result<ReleaseInstallResult, String> {
    let target_ip = request
        .target_ip_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let timeout_seconds = request.timeout_seconds.unwrap_or(180).clamp(30, 900);
    if target_ip.is_none() {
        emit_ota_step_progress(
            app,
            run_id_ref,
            "resolve-target",
            "info",
            "No OTA target IP was provided. Trying USB OTA recovery protocol.",
            None,
        );
        return install_helios_os_via_usb_recovery_ota(app, run_id_ref, &image_path, timeout_seconds);
    }
    let target_ip = target_ip.unwrap_or_default();
    let api_bases = ota_api_base_urls(target_ip);
    if api_bases.is_empty() {
        return Err(format!(
            "Unable to build OTA API endpoints for target '{target_ip}'."
        ));
    }

    emit_ota_step_progress(
        app,
        run_id_ref,
        "resolve-target",
        "running",
        format!("Resolving OTA target endpoint for {target_ip}."),
        None,
    );
    emit_ota_step_progress(
        app,
        run_id_ref,
        "resolve-target",
        "success",
        format!("Will try OTA API endpoints: {}.", api_bases.join(", ")),
        None,
    );

    let image_size_bytes = fs::metadata(&image_path)
        .ok()
        .map(|metadata| metadata.len())
        .filter(|value| *value > 0);

    fail_if_update_cancelled(app, run_id_ref, "ota", "ota-upload")?;
    emit_ota_step_progress_with_bytes(
        app,
        run_id_ref,
        "ota-upload",
        "running",
        "Uploading image to OTA endpoint.",
        Some(0.0),
        Some(0),
        image_size_bytes,
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|error| format!("Unable to build HTTP client for OTA: {error}"))?;

    let image_file_name = Path::new(&image_path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| "helios-update.img".to_string());

    let mut upload_errors = Vec::new();
    let mut uploaded: Option<OtaUploadResponse> = None;
    let mut successful_base: Option<String> = None;

    for (attempt_index, base) in api_bases.iter().enumerate() {
        fail_if_update_cancelled(app, run_id_ref, "ota", "ota-upload")?;
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-upload",
            "info",
            format!(
                "Trying OTA upload endpoint {}/{} at {base}.",
                attempt_index + 1,
                api_bases.len()
            ),
            None,
        );
        let upload_url = format!("{base}/v1/ota/upload");
        let upload_file = fs::File::open(&image_path)
            .map_err(|error| format!("Unable to open selected image for OTA upload: {error}"))?;
        let upload_reader = OtaUploadProgressReader::new(
            upload_file,
            app.clone(),
            run_id_ref.map(str::to_string),
            base.clone(),
            image_size_bytes,
        );
        let part = reqwest::blocking::multipart::Part::reader(upload_reader)
            .file_name(image_file_name.clone())
            .mime_str("application/octet-stream")
            .map_err(|error| format!("Unable to prepare OTA upload payload: {error}"))?;
        let form = reqwest::blocking::multipart::Form::new().part("file", part);
        let response = match client
            .post(&upload_url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .multipart(form)
            .send()
        {
            Ok(response) => response,
            Err(error) => {
                upload_errors.push(format!("{upload_url}: {error}"));
                continue;
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "<no response body>".to_string());
            upload_errors.push(format!("{upload_url}: HTTP {status} ({body})"));
            continue;
        }

        match response.json::<OtaUploadResponse>() {
            Ok(payload) => {
                uploaded = Some(payload);
                successful_base = Some(base.clone());
                break;
            }
            Err(error) => {
                upload_errors.push(format!("{upload_url}: invalid success payload ({error})"));
            }
        }
    }

    let Some(uploaded) = uploaded else {
        let network_error = format!(
            "OTA upload failed on all known API endpoints. {}",
            upload_errors.join(" | ")
        );
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-upload",
            "info",
            format!("{network_error} Attempting USB OTA recovery fallback."),
            None,
        );
        return match install_helios_os_via_usb_recovery_ota(
            app,
            run_id_ref,
            &image_path,
            timeout_seconds,
        ) {
            Ok(result) => Ok(result),
            Err(usb_error) => {
                let combined = format!(
                    "{network_error} USB OTA recovery fallback failed: {usb_error}"
                );
                emit_ota_step_progress(
                    app,
                    run_id_ref,
                    "ota-upload",
                    "error",
                    combined.clone(),
                    None,
                );
                Err(combined)
            }
        };
    };
    let Some(successful_base) = successful_base else {
        let error = "OTA upload completed but endpoint metadata was missing.".to_string();
        emit_ota_step_progress(app, run_id_ref, "ota-upload", "error", error.clone(), None);
        return Err(error);
    };

    let upload_summary = match (uploaded.filename.as_deref(), uploaded.size_bytes) {
        (Some(filename), Some(size_bytes)) => {
            format!("Uploaded OTA image '{filename}' ({size_bytes} bytes).")
        }
        (Some(filename), None) => format!("Uploaded OTA image '{filename}'."),
        _ => "Uploaded OTA image successfully.".to_string(),
    };
    let uploaded_size = uploaded.size_bytes.or(image_size_bytes);
    emit_ota_step_progress_with_bytes(
        app,
        run_id_ref,
        "ota-upload",
        "success",
        upload_summary,
        Some(100.0),
        uploaded_size,
        uploaded_size,
    );

    fail_if_update_cancelled(app, run_id_ref, "ota", "ota-apply")?;
    emit_ota_step_progress(
        app,
        run_id_ref,
        "ota-apply",
        "running",
        "Scheduling OTA apply operation on the device.",
        Some(5.0),
    );

    let apply_url = format!("{successful_base}/v1/ota/apply");
    let apply_payload = serde_json::json!({
        "requested_by": "Atlas Hardware Manager",
        "image_url": uploaded.image_url,
        "size_bytes": uploaded.size_bytes,
        "checksum": uploaded.sha256
    });
    let apply_response = client
        .post(&apply_url)
        .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
        .header(reqwest::header::ACCEPT, "application/json,text/plain,*/*")
        .json(&apply_payload)
        .send()
        .map_err(|error| format!("Failed to submit OTA apply request: {error}"))?;

    let apply_status = apply_response.status();
    if !apply_status.is_success() {
        let body = apply_response
            .text()
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "<no response body>".to_string());
        let error = format!("OTA apply failed: HTTP {apply_status} ({body})");
        emit_ota_step_progress(app, run_id_ref, "ota-apply", "error", error.clone(), None);
        return Err(error);
    }

    let apply_payload = apply_response
        .json::<OtaApplyResponse>()
        .ok()
        .unwrap_or(OtaApplyResponse {
            update_id: None,
            message: None,
        });
    let apply_message = apply_payload
        .message
        .unwrap_or_else(|| "OTA apply scheduled.".to_string());
    emit_ota_step_progress(
        app,
        run_id_ref,
        "ota-apply",
        "running",
        format!(
            "{apply_message} Device-side apply has started; monitoring OTA state for stage updates."
        ),
        Some(10.0),
    );
    persist_ota_recovery_update(
        app,
        run_id_ref,
        apply_payload.update_id.clone(),
        Some("OTA apply accepted by device. Waiting for reboot and reconnect.".to_string()),
    );

    let monitor_timeout_seconds = request.timeout_seconds.unwrap_or(360).clamp(120, 1800);
    let reconnect_message = wait_for_ota_reboot_and_reconnect(
        app,
        run_id_ref,
        target_ip,
        &api_bases,
        monitor_timeout_seconds,
        apply_payload.update_id.as_deref(),
    )?;
    emit_ota_step_progress(
        app,
        run_id_ref,
        "complete",
        "success",
        reconnect_message.clone(),
        Some(100.0),
    );

    let rpiboot =
        skipped_rpiboot_result("Skipped rpiboot because OTA update is being used for this device.");
    let update_id_suffix = apply_payload
        .update_id
        .map(|value| format!(" Update id: {value}."))
        .unwrap_or_default();
    Ok(ReleaseInstallResult {
        success: true,
        mode: "ota".to_string(),
        image_path: Some(image_path),
        selected_target_path: None,
        rpiboot,
        flash: None,
        message: format!("{reconnect_message}{update_id_suffix}"),
    })
}

fn install_helios_os_via_usb_recovery_ota(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    image_path: &str,
    timeout_seconds: u64,
) -> Result<ReleaseInstallResult, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, run_id_ref, image_path, timeout_seconds);
        return Err(
            "USB OTA recovery protocol fallback is currently supported on Linux hosts only."
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        use base64::Engine;

        fail_if_update_cancelled(app, run_id_ref, "ota", "resolve-target")?;
        emit_ota_step_progress(
            app,
            run_id_ref,
            "resolve-target",
            "running",
            "Probing USB recovery serial endpoints.",
            None,
        );
        let request_timeout = Duration::from_secs(timeout_seconds.clamp(10, 900));
        let mut usb_client = connect_usb_recovery_client(app, run_id_ref, request_timeout)?;
        emit_ota_step_progress(
            app,
            run_id_ref,
            "resolve-target",
            "success",
            format!(
                "Using USB OTA recovery protocol on serial endpoint {}.",
                usb_client.port_label()
            ),
            None,
        );
        ensure_no_active_usb_ota_transfer(app, run_id_ref, &mut usb_client)?;

        let image_path_buf = Path::new(image_path);
        let image_size_bytes = fs::metadata(image_path_buf)
            .map(|metadata| metadata.len())
            .map_err(|error| format!("Unable to stat selected image for USB OTA: {error}"))?;
        if image_size_bytes == 0 {
            return Err("Selected image is empty and cannot be uploaded over USB OTA.".to_string());
        }
        let image_sha256 = hash_file_sha256(image_path_buf)?;
        let image_file_name = image_path_buf
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| "helios-update.img".to_string());

        fail_if_update_cancelled(app, run_id_ref, "ota", "ota-upload")?;
        emit_ota_step_progress_with_bytes(
            app,
            run_id_ref,
            "ota-upload",
            "running",
            "Uploading OTA image over USB recovery protocol.",
            Some(0.0),
            Some(0),
            Some(image_size_bytes),
        );
        let begin_result = begin_usb_ota_transfer_with_stale_retry(
            app,
            run_id_ref,
            &mut usb_client,
            image_size_bytes,
            &image_sha256,
            &image_file_name,
        )?;
        let transfer_id = begin_result
            .get("transfer_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "USB OTA begin response missing transfer_id.".to_string())?;
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-upload",
            "info",
            format!("USB OTA transfer started (transfer {transfer_id})."),
            None,
        );

        let mut image_file = fs::File::open(image_path_buf)
            .map_err(|error| format!("Unable to open selected image for USB OTA upload: {error}"))?;
        let mut buffer = vec![0u8; 49_152];
        let mut offset = 0u64;
        let mut last_emitted_bytes = 0u64;
        loop {
            fail_if_update_cancelled(app, run_id_ref, "ota", "ota-upload")?;
            let read = image_file
                .read(&mut buffer)
                .map_err(|error| format!("Failed reading OTA image during USB upload: {error}"))?;
            if read == 0 {
                break;
            }
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(&buffer[..read]);
            let chunk_result = usb_client.call(
                "ota.chunk",
                serde_json::json!({
                    "transfer_id": transfer_id,
                    "offset": offset,
                    "data_b64": encoded,
                }),
            )?;
            offset = chunk_result
                .get("next_offset")
                .and_then(Value::as_u64)
                .ok_or_else(|| "USB OTA chunk response missing next_offset.".to_string())?;
            if offset == image_size_bytes
                || offset.saturating_sub(last_emitted_bytes) >= 8 * 1024 * 1024
            {
                last_emitted_bytes = offset;
                let percent =
                    ((offset as f64 / image_size_bytes as f64) * 100.0).clamp(0.0, 100.0);
                emit_ota_step_progress_with_bytes(
                    app,
                    run_id_ref,
                    "ota-upload",
                    "running",
                    format!(
                        "Uploading OTA image over USB recovery: {:.1}% ({offset}/{image_size_bytes} bytes).",
                        percent
                    ),
                    Some(percent),
                    Some(offset),
                    Some(image_size_bytes),
                );
            }
        }
        let finish_result = usb_client.call(
            "ota.finish",
            serde_json::json!({ "transfer_id": transfer_id }),
        )?;
        let staged_path = finish_result
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("<device staging>");
        emit_ota_step_progress_with_bytes(
            app,
            run_id_ref,
            "ota-upload",
            "success",
            format!(
                "USB OTA upload verified on device (transfer {transfer_id}, path {staged_path})."
            ),
            Some(100.0),
            Some(image_size_bytes),
            Some(image_size_bytes),
        );

        fail_if_update_cancelled(app, run_id_ref, "ota", "ota-apply")?;
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-apply",
            "running",
            "Activating USB OTA image and requesting reboot.",
            Some(10.0),
        );
        let activate_result = usb_client.call(
            "ota.activate",
            serde_json::json!({
                "transfer_id": transfer_id,
                "reboot": true,
            }),
        )?;
        let reboot_scheduled = activate_result
            .get("reboot_scheduled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-apply",
            "success",
            if reboot_scheduled {
                format!(
                    "USB OTA activate accepted for transfer {transfer_id}. Reboot scheduled."
                )
            } else {
                format!(
                    "USB OTA activate accepted for transfer {transfer_id}. Device did not report scheduled reboot."
                )
            },
            Some(100.0),
        );
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-monitor",
            "success",
            "USB OTA recovery acknowledged upload and activate commands.",
            Some(100.0),
        );
        let reconnect_message = if reboot_scheduled {
            "USB OTA update applied. Device reboot was requested; wait for reconnect."
                .to_string()
        } else {
            "USB OTA update applied. Device reboot may still be required depending on target configuration."
                .to_string()
        };
        emit_ota_step_progress(
            app,
            run_id_ref,
            "ota-reconnect",
            "success",
            reconnect_message.clone(),
            Some(100.0),
        );
        emit_ota_step_progress(
            app,
            run_id_ref,
            "complete",
            "success",
            reconnect_message.clone(),
            Some(100.0),
        );
        let rpiboot = skipped_rpiboot_result(
            "Skipped rpiboot because USB OTA recovery protocol was used for this device.",
        );
        Ok(ReleaseInstallResult {
            success: true,
            mode: "ota".to_string(),
            image_path: Some(image_path.to_string()),
            selected_target_path: None,
            rpiboot,
            flash: None,
            message: reconnect_message,
        })
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
struct UsbRecoveryResponseError {
    code: String,
    message: String,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
struct UsbRecoveryResponse {
    id: Value,
    ok: bool,
    result: Option<Value>,
    error: Option<UsbRecoveryResponseError>,
}

#[cfg(target_os = "linux")]
struct UsbRecoveryClient {
    port: fs::File,
    port_path: PathBuf,
    timeout: Duration,
    next_id: u64,
}

#[cfg(target_os = "linux")]
enum UsbRecoveryOpenError {
    PermissionDenied(String),
    Other(String),
}

#[cfg(target_os = "linux")]
impl UsbRecoveryClient {
    fn open(path: &Path, timeout: Duration) -> Result<Self, UsbRecoveryOpenError> {
        use std::os::unix::fs::OpenOptionsExt;

        let port = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::PermissionDenied {
                    UsbRecoveryOpenError::PermissionDenied(format!(
                        "Unable to open USB recovery serial endpoint {}: {error}. \
Linux denied access to the serial device. Add your user to the `dialout` group and sign out/in \
(`sudo usermod -aG dialout $USER`), or run Atlas with elevated privileges.",
                        path.display()
                    ))
                } else {
                    UsbRecoveryOpenError::Other(format!(
                        "Unable to open USB recovery serial endpoint {}: {error}",
                        path.display()
                    ))
                }
            })?;

        Ok(Self {
            port,
            port_path: path.to_path_buf(),
            timeout,
            next_id: 1,
        })
    }

    fn port_label(&self) -> String {
        self.port_path.display().to_string()
    }

    fn call(&mut self, op: &str, params: Value) -> Result<Value, String> {
        let request_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = serde_json::json!({
            "id": request_id,
            "op": op,
            "params": params,
        });
        let request_text = serde_json::to_string(&request)
            .map_err(|error| format!("Failed to encode USB OTA request `{op}`: {error}"))?;
        self.port
            .write_all(request_text.as_bytes())
            .map_err(|error| format!("Failed to write USB OTA request `{op}`: {error}"))?;
        self.port
            .write_all(b"\n")
            .map_err(|error| format!("Failed to finish USB OTA request `{op}`: {error}"))?;
        self.port
            .flush()
            .map_err(|error| format!("Failed to flush USB OTA request `{op}`: {error}"))?;

        let started = Instant::now();
        let mut read_buf = [0u8; 4096];
        let mut buffer = Vec::<u8>::new();
        while started.elapsed() < self.timeout {
            match self.port.read(&mut read_buf) {
                Ok(0) => {}
                Ok(bytes_read) => {
                    buffer.extend_from_slice(&read_buf[..bytes_read]);
                    if let Some(newline_idx) = buffer.iter().position(|byte| *byte == b'\n') {
                        let mut line = buffer.drain(..=newline_idx).collect::<Vec<u8>>();
                        while matches!(line.last(), Some(b'\n' | b'\r')) {
                            let _ = line.pop();
                        }
                        if line.is_empty() {
                            continue;
                        }

                        let response = serde_json::from_slice::<UsbRecoveryResponse>(&line)
                            .map_err(|error| {
                                format!(
                                    "Failed parsing USB OTA response for `{op}` as JSON: {error}"
                                )
                            })?;
                        if !usb_recovery_response_id_matches(
                            Some(&response.id),
                            request_id,
                        ) {
                            continue;
                        }

                        if response.ok {
                            return response.result.ok_or_else(|| {
                                format!("USB OTA response for `{op}` did not include a result.")
                            });
                        }
                        let error = response.error.ok_or_else(|| {
                            format!(
                                "USB OTA request `{op}` failed but did not include structured error details."
                            )
                        })?;
                        return Err(format!(
                            "USB OTA request `{op}` failed ({}): {}",
                            error.code, error.message
                        ));
                    }
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(format!("Failed reading USB OTA response for `{op}`: {error}"));
                }
            }
            if buffer.len() > 4 * 1024 * 1024 {
                return Err(
                    "USB OTA response exceeded 4 MiB without newline termination.".to_string(),
                );
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err(format!(
            "Timed out waiting for USB OTA response to `{op}` after {}s.",
            self.timeout.as_secs()
        ))
    }
}

#[cfg(target_os = "linux")]
fn usb_recovery_response_id_matches(response_id: Option<&Value>, request_id: u64) -> bool {
    let Some(value) = response_id else {
        return true;
    };
    if let Some(id_number) = value.as_u64() {
        return id_number == request_id;
    }
    if let Some(id_text) = value.as_str() {
        return id_text == request_id.to_string();
    }
    true
}

#[cfg(target_os = "linux")]
fn ensure_no_active_usb_ota_transfer(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    client: &mut UsbRecoveryClient,
) -> Result<(), String> {
    let status = client
        .call("status.get", serde_json::json!({}))
        .map_err(|error| format!("Unable to query USB OTA status before upload: {error}"))?;
    if !usb_ota_status_in_progress(&status) {
        return Ok(());
    }

    emit_ota_step_progress(
        app,
        run_id_ref,
        "resolve-target",
        "info",
        "Detected an active USB OTA transfer from a previous attempt. Aborting it before starting a new upload.",
        None,
    );
    let active_transfer_id = usb_ota_status_active_transfer_id(&status);
    let abort_result = abort_active_usb_ota_transfer(client, active_transfer_id.as_deref())?;
    let aborted = abort_result
        .get("aborted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let reason = abort_result
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    emit_ota_step_progress(
        app,
        run_id_ref,
        "resolve-target",
        "info",
        if aborted {
            "Aborted stale USB OTA transfer.".to_string()
        } else {
            format!("No active USB OTA transfer was aborted ({reason}).")
        },
        None,
    );

    let post_status = client
        .call("status.get", serde_json::json!({}))
        .map_err(|error| format!("Unable to verify USB OTA transfer state after abort: {error}"))?;
    if usb_ota_status_in_progress(&post_status) {
        return Err(
            "USB OTA transfer is still active after abort attempt. Retry after reconnecting USB."
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn begin_usb_ota_transfer_with_stale_retry(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    client: &mut UsbRecoveryClient,
    image_size_bytes: u64,
    image_sha256: &str,
    image_file_name: &str,
) -> Result<Value, String> {
    let begin_params = serde_json::json!({
        "size": image_size_bytes,
        "sha256": image_sha256,
        "version": image_file_name,
    });
    match client.call("ota.begin", begin_params.clone()) {
        Ok(result) => Ok(result),
        Err(error) => {
            if !usb_ota_begin_failed_due_to_active_transfer(&error) {
                return Err(error);
            }
            emit_ota_step_progress(
                app,
                run_id_ref,
                "ota-upload",
                "info",
                "USB OTA begin reported an active transfer. Aborting stale transfer and retrying once.",
                None,
            );
            let abort_result = abort_active_usb_ota_transfer(client, None)?;
            let aborted = abort_result
                .get("aborted")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            emit_ota_step_progress(
                app,
                run_id_ref,
                "ota-upload",
                "info",
                if aborted {
                    "Stale USB OTA transfer aborted; retrying ota.begin.".to_string()
                } else {
                    "No active USB OTA transfer was aborted; retrying ota.begin.".to_string()
                },
                None,
            );
            client.call("ota.begin", begin_params).map_err(|retry_error| {
                format!(
                    "USB OTA begin retry failed after stale-transfer abort: {retry_error}"
                )
            })
        }
    }
}

#[cfg(target_os = "linux")]
fn abort_active_usb_ota_transfer(
    client: &mut UsbRecoveryClient,
    transfer_id: Option<&str>,
) -> Result<Value, String> {
    let params = match transfer_id {
        Some(id) if !id.trim().is_empty() => serde_json::json!({ "transfer_id": id }),
        _ => Value::Null,
    };
    client.call("ota.abort", params)
}

#[cfg(target_os = "linux")]
fn usb_ota_status_in_progress(status: &Value) -> bool {
    status
        .get("ota")
        .and_then(Value::as_object)
        .and_then(|ota| ota.get("in_progress"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn usb_ota_status_active_transfer_id(status: &Value) -> Option<String> {
    status
        .get("ota")
        .and_then(Value::as_object)
        .and_then(|ota| ota.get("active_transfer_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(target_os = "linux")]
fn usb_ota_begin_failed_due_to_active_transfer(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("ota.begin")
        && (normalized.contains("already active")
            || normalized.contains("active ota transfer")
            || normalized.contains("bad_state"))
}

#[cfg(target_os = "linux")]
fn connect_usb_recovery_client(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    timeout: Duration,
) -> Result<UsbRecoveryClient, String> {
    let mut candidates = Vec::<PathBuf>::new();
    if let Ok(override_path) = env::var("HELIOS_USB_OTA_PORT") {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }
    candidates.extend(discover_usb_recovery_ports());
    candidates.sort();
    candidates.dedup();

    if candidates.is_empty() {
        return Err(
            "No USB recovery serial endpoints were detected. Connect the device over USB and retry."
                .to_string(),
        );
    }

    let mut last_error: Option<String> = None;
    let mut elevation_attempted = false;
    for path in &candidates {
        let mut client = match UsbRecoveryClient::open(path, timeout) {
            Ok(client) => client,
            Err(UsbRecoveryOpenError::PermissionDenied(error)) => {
                if elevation_attempted {
                    last_error = Some(error);
                    continue;
                }
                elevation_attempted = true;
                emit_ota_step_progress(
                    app,
                    run_id_ref,
                    "resolve-target",
                    "info",
                    "USB recovery serial endpoint requires host elevation. Requesting one-time elevated permission.",
                    None,
                );
                let elevation_result = grant_usb_recovery_serial_access_with_elevation(&candidates);
                match elevation_result {
                    Ok(method) => {
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "resolve-target",
                            "info",
                            format!(
                                "USB recovery serial access granted via {method}. Retrying endpoint open."
                            ),
                            None,
                        );
                        match UsbRecoveryClient::open(path, timeout) {
                            Ok(client) => client,
                            Err(UsbRecoveryOpenError::PermissionDenied(retry_error))
                            | Err(UsbRecoveryOpenError::Other(retry_error)) => {
                                last_error = Some(retry_error);
                                continue;
                            }
                        }
                    }
                    Err(elevation_error) => {
                        last_error = Some(format!("{error} {elevation_error}"));
                        continue;
                    }
                }
            }
            Err(UsbRecoveryOpenError::Other(error)) => {
                last_error = Some(error);
                continue;
            }
        };
        match client.call("hello", serde_json::json!({})) {
            Ok(result) => {
                let protocol = result
                    .get("protocol")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if protocol.starts_with("helios-usb-recovery") {
                    return Ok(client);
                }
                match client.call("ping", serde_json::json!({})) {
                    Ok(ping) if ping.get("pong").and_then(Value::as_bool) == Some(true) => {
                        return Ok(client);
                    }
                    Ok(_) => {
                        last_error = Some(format!(
                            "Serial endpoint {} responded but is not a HeliOS USB recovery service.",
                            path.display()
                        ));
                    }
                    Err(error) => {
                        last_error = Some(error);
                    }
                }
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        "Unable to connect to a HeliOS USB recovery endpoint over serial.".to_string()
    }))
}

#[cfg(target_os = "linux")]
fn grant_usb_recovery_serial_access_with_elevation(paths: &[PathBuf]) -> Result<String, String> {
    if paths.is_empty() {
        return Err("No USB recovery serial endpoints were available for permission elevation.".to_string());
    }

    let strategy = linux_privilege_strategy()?;
    let timeout = Duration::from_secs(30);
    let path_args = paths
        .iter()
        .map(|path| path_to_string(path.as_path()))
        .collect::<Vec<_>>();
    let (command_program, command_args) = if let Some(setfacl_tool) = resolve_tool("setfacl") {
        let uid = unsafe { libc::geteuid() };
        let mut args = vec!["-m".to_string(), format!("u:{uid}:rw")];
        args.extend(path_args.iter().cloned());
        (path_to_string(&setfacl_tool.path), args)
    } else {
        let chmod_path = resolve_required_tool("chmod")?;
        let mut args = vec!["a+rw".to_string()];
        args.extend(path_args.iter().cloned());
        (path_to_string(&chmod_path), args)
    };

    let output = run_linux_privileged_tool_command(
        strategy,
        &command_program,
        &command_args,
        timeout,
    )?;
    if !output.output.status.success() {
        let stderr = trim_output(&output.output.stderr);
        let stdout = trim_output(&output.output.stdout);
        return Err(format!(
            "Failed to grant temporary USB serial permissions. stderr: {stderr}; stdout: {stdout}"
        ));
    }

    Ok(match strategy {
        LinuxPrivilegeStrategy::Direct => "direct host privileges".to_string(),
        LinuxPrivilegeStrategy::SudoNoPrompt => "sudo (cached/no-prompt)".to_string(),
        LinuxPrivilegeStrategy::PkexecPrompt => "pkexec".to_string(),
        LinuxPrivilegeStrategy::SudoPrompt => "sudo".to_string(),
    })
}

#[cfg(target_os = "linux")]
fn run_linux_privileged_tool_command(
    strategy: LinuxPrivilegeStrategy,
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<TimedCommandOutput, String> {
    match strategy {
        LinuxPrivilegeStrategy::Direct => {
            let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            run_command_with_timeout(program, &arg_refs, timeout)
        }
        LinuxPrivilegeStrategy::SudoNoPrompt => {
            let mut sudo_args = vec!["-n".to_string(), program.to_string()];
            sudo_args.extend(args.iter().cloned());
            let arg_refs = sudo_args.iter().map(String::as_str).collect::<Vec<_>>();
            run_command_with_timeout("sudo", &arg_refs, timeout)
        }
        LinuxPrivilegeStrategy::PkexecPrompt => {
            let mut pkexec_args = vec![program.to_string()];
            pkexec_args.extend(args.iter().cloned());
            let arg_refs = pkexec_args.iter().map(String::as_str).collect::<Vec<_>>();
            run_command_with_timeout("pkexec", &arg_refs, timeout)
        }
        LinuxPrivilegeStrategy::SudoPrompt => {
            let mut sudo_args = Vec::<String>::new();
            let askpass_path = resolve_sudo_askpass_helper();
            if askpass_path.is_some() {
                sudo_args.push("-A".to_string());
            }
            sudo_args.push(program.to_string());
            sudo_args.extend(args.iter().cloned());
            let arg_refs = sudo_args.iter().map(String::as_str).collect::<Vec<_>>();
            if let Some(path) = askpass_path.as_ref() {
                let askpass_path_string = path_to_string(path);
                run_command_with_timeout_and_env(
                    "sudo",
                    &arg_refs,
                    &[("SUDO_ASKPASS", askpass_path_string.as_str())],
                    timeout,
                )
            } else {
                run_command_with_timeout("sudo", &arg_refs, timeout)
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn discover_usb_recovery_ports() -> Vec<PathBuf> {
    let mut ports = fs::read_dir("/dev")
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    name.starts_with("ttyACM")
                        || name.starts_with("cu.usbmodem")
                        || name.starts_with("tty.usbmodem")
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    ports.sort();
    ports
}

#[cfg(target_os = "linux")]
fn hash_file_sha256(path: &Path) -> Result<String, String> {
    use sha2::Digest;

    let mut file = fs::File::open(path)
        .map_err(|error| format!("Unable to open image for SHA-256 hashing: {error}"))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Unable to read image while hashing: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn ota_api_base_urls(target_ip: &str) -> Vec<String> {
    let mut candidates = vec![
        format!("http://{target_ip}:5801"),
        format!("http://{target_ip}:5800"),
        format!("http://{target_ip}:80"),
        format!("http://{target_ip}"),
    ];
    candidates.retain(|value| !value.trim().is_empty());
    let mut deduped = Vec::new();
    for candidate in candidates {
        if !deduped.contains(&candidate) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn wait_for_ota_reboot_and_reconnect(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    target_ip: &str,
    api_bases: &[String],
    timeout_seconds: u64,
    expected_update_id: Option<&str>,
) -> Result<String, String> {
    let poll_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| format!("Unable to build OTA polling HTTP client: {error}"))?;
    let probe_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1800))
        .build()
        .map_err(|error| format!("Unable to build runtime probe HTTP client: {error}"))?;

    emit_ota_step_progress(
        app,
        run_id_ref,
        "ota-monitor",
        "running",
        "Monitoring OTA state until reboot starts.",
        Some(0.0),
    );

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_seconds);
    let mut last_stage: Option<String> = None;
    let mut last_percent_bucket: Option<i32> = None;
    let mut last_wait_notice = Instant::now() - Duration::from_secs(30);
    let mut monitor_success_emitted = false;
    let mut reconnect_running_emitted = false;
    let mut saw_apply_related_stage = false;
    let mut saw_reboot_stage = false;
    let mut saw_offline_after_apply = false;
    let mut consecutive_online_checks = 0u8;
    let mut apply_running_emitted = false;
    let mut apply_success_emitted = false;
    let mut last_apply_stage: Option<String> = None;
    let mut last_apply_percent_bucket: Option<i32> = None;

    loop {
        fail_if_update_cancelled(
            app,
            run_id_ref,
            "ota",
            if reconnect_running_emitted {
                "ota-reconnect"
            } else {
                "ota-monitor"
            },
        )?;
        if start.elapsed() > timeout {
            return Err(format!(
                "Timed out waiting for OTA reboot/reconnect after {}s. Last stage: {}.",
                timeout_seconds,
                last_stage.as_deref().unwrap_or("unknown")
            ));
        }

        match fetch_ota_runtime_state(&poll_client, api_bases) {
            Ok(state) => {
                let stage = state
                    .stage
                    .unwrap_or_else(|| "idle".to_string())
                    .trim()
                    .to_ascii_lowercase();
                let progress = state
                    .progress_percent
                    .or_else(|| ota_stage_progress_hint(&stage))
                    .map(|value| value.clamp(0.0, 100.0));
                let percent_bucket = progress.map(|value| (value / 5.0).floor() as i32);
                let last_error = state
                    .last_error
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);

                let update_id_label = state
                    .update_id
                    .as_deref()
                    .or(expected_update_id)
                    .unwrap_or("unknown");
                if last_stage.as_deref() != Some(stage.as_str())
                    || last_percent_bucket != percent_bucket
                {
                    let mut detail = format!("OTA state: {stage} (update {update_id_label}).");
                    if let Some(error) = last_error.as_deref() {
                        detail = format!("{detail} Last error: {error}");
                    }
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-monitor",
                        "running",
                        detail,
                        progress,
                    );
                    last_stage = Some(stage.clone());
                    last_percent_bucket = percent_bucket;
                }

                if is_ota_apply_stage(&stage) {
                    saw_apply_related_stage = true;
                    let apply_progress = progress
                        .or_else(|| ota_apply_stage_progress_hint(&stage))
                        .map(|value| value.clamp(0.0, 100.0));
                    let apply_percent_bucket =
                        apply_progress.map(|value| (value / 2.0).floor() as i32);
                    if !apply_success_emitted
                        && (!apply_running_emitted
                            || last_apply_stage.as_deref() != Some(stage.as_str())
                            || last_apply_percent_bucket != apply_percent_bucket)
                    {
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-apply",
                            "running",
                            format!("OTA apply stage: {stage}."),
                            apply_progress,
                        );
                        apply_running_emitted = true;
                        last_apply_stage = Some(stage.clone());
                        last_apply_percent_bucket = apply_percent_bucket;
                    }
                }
                if stage == "rebooting" || stage == "complete" {
                    saw_reboot_stage = true;
                }
                if !apply_success_emitted && (stage == "rebooting" || stage == "complete") {
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-apply",
                        "success",
                        "Device entered reboot/finalization phase for OTA apply.",
                        progress,
                    );
                    apply_success_emitted = true;
                }
                if is_ota_terminal_error_stage(&stage) {
                    let error_reason = last_error
                        .clone()
                        .unwrap_or_else(|| format!("OTA apply failed during '{stage}' stage."));
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-apply",
                        "error",
                        error_reason.clone(),
                        progress,
                    );
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-monitor",
                        "error",
                        error_reason.clone(),
                        progress,
                    );
                    return Err(error_reason);
                }
                if let Some(error_text) = last_error.as_deref() {
                    if is_ota_out_of_space_error(error_text)
                        || (saw_apply_related_stage && stage == "idle")
                    {
                        let error_reason = format!("OTA apply failed: {error_text}");
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-apply",
                            "error",
                            error_reason.clone(),
                            progress,
                        );
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-monitor",
                            "error",
                            error_reason.clone(),
                            progress,
                        );
                        return Err(error_reason);
                    }
                }

                if !monitor_success_emitted && (saw_reboot_stage || saw_offline_after_apply) {
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-monitor",
                        "success",
                        "OTA apply reached reboot phase; waiting for device to return online.",
                        Some(100.0),
                    );
                    monitor_success_emitted = true;
                }

                if (saw_offline_after_apply || saw_reboot_stage)
                    && is_device_runtime_online(&probe_client, target_ip)
                {
                    if !reconnect_running_emitted {
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-reconnect",
                            "running",
                            "Device responded after reboot. Verifying stable connectivity.",
                            Some(60.0),
                        );
                        reconnect_running_emitted = true;
                    }
                    consecutive_online_checks = consecutive_online_checks.saturating_add(1);
                    if consecutive_online_checks >= 2 {
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-reconnect",
                            "success",
                            format!(
                                "Device rebooted and is back online at {target_ip}. OTA workflow complete."
                            ),
                            Some(100.0),
                        );
                        return Ok(format!(
                            "OTA apply scheduled and completed. Device rebooted and is back online at {target_ip}."
                        ));
                    }
                } else {
                    consecutive_online_checks = 0;
                }
            }
            Err(error) => {
                consecutive_online_checks = 0;
                let treat_as_offline = ota_poll_error_indicates_offline(&error);
                if is_ota_out_of_space_error(&error) {
                    let error_reason = format!(
                        "OTA apply failed: device reported insufficient storage. {error}"
                    );
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-apply",
                        "error",
                        error_reason.clone(),
                        Some(0.0),
                    );
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-monitor",
                        "error",
                        error_reason.clone(),
                        Some(0.0),
                    );
                    return Err(error_reason);
                }
                if saw_apply_related_stage || saw_reboot_stage {
                    if !treat_as_offline {
                        let error_reason = format!(
                            "OTA state polling failed during apply: {error}"
                        );
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-apply",
                            "error",
                            error_reason.clone(),
                            Some(0.0),
                        );
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-monitor",
                            "error",
                            error_reason.clone(),
                            Some(0.0),
                        );
                        return Err(error_reason);
                    }
                    if !saw_offline_after_apply {
                        saw_offline_after_apply = true;
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-monitor",
                            "running",
                            "OTA endpoint went offline, indicating reboot started.",
                            Some(100.0),
                        );
                    }
                    if !monitor_success_emitted {
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-monitor",
                            "success",
                            "OTA apply reached reboot phase; waiting for device to return online.",
                            Some(100.0),
                        );
                        monitor_success_emitted = true;
                    }
                    if !reconnect_running_emitted {
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-reconnect",
                            "running",
                            "Device is rebooting and currently offline. Waiting for it to come back.",
                            Some(10.0),
                        );
                        reconnect_running_emitted = true;
                    }
                } else if last_wait_notice.elapsed() >= Duration::from_secs(20) {
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-monitor",
                        "info",
                        format!("Waiting for OTA state endpoint to report progress: {error}"),
                        None,
                    );
                    last_wait_notice = Instant::now();
                }
            }
        }

        thread::sleep(Duration::from_secs(2));
    }
}

fn fetch_ota_runtime_state(
    client: &reqwest::blocking::Client,
    api_bases: &[String],
) -> Result<OtaRuntimeState, String> {
    let mut errors = Vec::new();
    for base in api_bases {
        let url = format!("{base}/v1/ota/state");
        let response = match client
            .get(&url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .header(reqwest::header::ACCEPT, "application/json,text/plain,*/*")
            .send()
        {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!("{url}: {error}"));
                continue;
            }
        };
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "<no response body>".to_string());
            errors.push(format!("{url}: HTTP {status} ({body})"));
            continue;
        }
        let raw_state = match response.json::<Value>() {
            Ok(value) => value,
            Err(error) => {
                errors.push(format!("{url}: invalid JSON response ({error})"));
                continue;
            }
        };
        return Ok(parse_ota_runtime_state(Some(&raw_state)));
    }

    if errors.is_empty() {
        Err("No OTA state endpoints were available.".to_string())
    } else {
        Err(errors.join(" | "))
    }
}

fn parse_ota_runtime_state(raw: Option<&Value>) -> OtaRuntimeState {
    let Some(raw) = raw else {
        return OtaRuntimeState::default();
    };
    let root = match raw {
        Value::Object(root) => root,
        _ => return OtaRuntimeState::default(),
    };

    let read_text_from =
        |source: &serde_json::Map<String, Value>, keys: &[&str]| -> Option<String> {
            for key in keys {
                if let Some(value) = source.get(*key) {
                    let parsed = match value {
                        Value::String(text) => Some(text.trim().to_string()),
                        Value::Number(number) => Some(number.to_string()),
                        Value::Bool(flag) => Some(flag.to_string()),
                        _ => None,
                    }
                    .filter(|text| !text.is_empty());
                    if parsed.is_some() {
                        return parsed;
                    }
                }
            }
            None
        };
    let read_progress_from =
        |source: &serde_json::Map<String, Value>, keys: &[&str]| -> Option<f64> {
            for key in keys {
                if let Some(value) = source.get(*key) {
                    if let Some(parsed) = parse_ota_runtime_progress_value(value, key) {
                        return Some(parsed);
                    }
                }
            }
            None
        };

    let sources = collect_ota_runtime_state_sources(root);

    let read_text = |keys: &[&str]| -> Option<String> {
        for source in &sources {
            if let Some(value) = read_text_from(source, keys) {
                return Some(value);
            }
        }
        None
    };
    let read_progress = |keys: &[&str]| -> Option<f64> {
        for source in &sources {
            if let Some(value) = read_progress_from(source, keys) {
                return Some(value);
            }
        }
        None
    };

    OtaRuntimeState {
        update_id: read_text(&["update_id", "updateId", "id"]),
        stage: read_text(&[
            "stage",
            "status",
            "phase",
            "state",
            "state_name",
            "stateName",
            "update_stage",
            "updateStage",
            "apply_state",
            "applyState",
            "ota_stage",
            "otaStage",
            "current_stage",
            "currentStage",
        ])
        .map(normalize_ota_stage_name),
        progress_percent: read_progress(&[
            "progress_percent",
            "progressPercent",
            "progress",
            "percent",
            "pct",
            "stage_percent",
            "stagePercent",
            "apply_progress",
            "applyProgress",
            "apply_percent",
            "applyPercent",
            "progress_ratio",
            "progressRatio",
            "progress_fraction",
            "progressFraction",
            "fraction_complete",
            "fractionComplete",
        ])
        .map(|value| value.clamp(0.0, 100.0)),
        last_error: read_text(&[
            "last_error",
            "lastError",
            "error",
            "error_message",
            "errorMessage",
            "reason",
            "detail",
            "message",
            "failure_reason",
            "failureReason",
        ]),
    }
}

fn collect_ota_runtime_state_sources(
    root: &serde_json::Map<String, Value>,
) -> Vec<&serde_json::Map<String, Value>> {
    const CHILD_KEYS: [&str; 15] = [
        "state",
        "active_update",
        "activeUpdate",
        "active",
        "ota",
        "update",
        "updater",
        "snapshot",
        "payload",
        "result",
        "data",
        "runtime",
        "status",
        "progress",
        "apply_progress",
    ];

    let mut sources = Vec::<&serde_json::Map<String, Value>>::new();
    let mut queue = vec![root];
    let mut seen = HashSet::<usize>::new();
    while let Some(source) = queue.pop() {
        let ptr = source as *const _ as usize;
        if !seen.insert(ptr) {
            continue;
        }
        sources.push(source);
        for key in CHILD_KEYS {
            if let Some(Value::Object(child)) = source.get(key) {
                queue.push(child);
            }
        }
    }
    sources
}

fn parse_ota_runtime_progress_value(value: &Value, key: &str) -> Option<f64> {
    let mut parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_ota_runtime_number_text(text),
        _ => None,
    }?;
    if !parsed.is_finite() {
        return None;
    }

    let key_lower = key.to_ascii_lowercase();
    let percent_key =
        key_lower.contains("percent") || key_lower == "pct" || key_lower.ends_with("_pct");
    if (0.0..=1.0).contains(&parsed) && (!percent_key || parsed.fract() > 0.0) {
        parsed *= 100.0;
    }
    Some(parsed)
}

fn parse_ota_runtime_number_text(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed
        .strip_suffix('%')
        .unwrap_or(trimmed)
        .trim();
    if normalized.is_empty() {
        return None;
    }
    normalized.parse::<f64>().ok()
}

fn normalize_ota_stage_name(stage: String) -> String {
    let normalized = stage.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return normalized;
    }
    let compact = normalized.replace(['-', ' '], "_");
    match compact.as_str() {
        "download" => "downloading".to_string(),
        "awaitingwindow" | "scheduled" | "apply_scheduled" | "queued" => {
            "awaiting_window".to_string()
        }
        "in_progress" | "apply_in_progress" | "install_in_progress" | "install" => {
            "applying".to_string()
        }
        "pendingreboot" | "awaiting_reboot" | "reboot_required" => "pending_reboot".to_string(),
        "finalized" => "finalizing".to_string(),
        "done" | "success" | "succeeded" => "complete".to_string(),
        "rolledback" | "rolling_back" => "rolled_back".to_string(),
        "canceled" => "cancelled".to_string(),
        _ if compact.starts_with("failed") || compact.ends_with("_failed") => {
            "failed".to_string()
        }
        _ => compact,
    }
}

#[cfg(test)]
mod updater_install_flow_tests {
    use super::*;

    #[test]
    fn parse_ota_state_from_http_state_wrapper() {
        let payload = serde_json::json!({
            "state": {
                "update_id": "11111111-2222-3333-4444-555555555555",
                "stage": "applying",
                "progress_percent": 42,
                "last_error": null
            },
            "cache_usage_bytes": 1234
        });
        let parsed = parse_ota_runtime_state(Some(&payload));
        assert_eq!(
            parsed.update_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(parsed.stage.as_deref(), Some("applying"));
        assert_eq!(parsed.progress_percent, Some(42.0));
        assert_eq!(parsed.last_error, None);
    }

    #[test]
    fn parse_ota_state_from_active_update_wrapper_with_fractional_progress() {
        let payload = serde_json::json!({
            "active_update": {
                "updateId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                "stateName": "AwaitingWindow",
                "progress": "0.58"
            }
        });
        let parsed = parse_ota_runtime_state(Some(&payload));
        assert_eq!(
            parsed.update_id.as_deref(),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
        );
        assert_eq!(parsed.stage.as_deref(), Some("awaiting_window"));
        let progress = parsed.progress_percent.unwrap_or_default();
        assert!((progress - 58.0).abs() < 0.001);
    }

    #[test]
    fn parse_ota_state_from_nested_progress_percent_string() {
        let payload = serde_json::json!({
            "result": {
                "ota": {
                    "update_stage": "apply_in_progress",
                    "progress": {
                        "percent": "33%"
                    }
                }
            }
        });
        let parsed = parse_ota_runtime_state(Some(&payload));
        assert_eq!(parsed.stage.as_deref(), Some("applying"));
        assert_eq!(parsed.progress_percent, Some(33.0));
    }
}

fn is_ota_apply_stage(stage: &str) -> bool {
    matches!(
        stage,
        "downloading"
            | "verifying"
            | "awaiting_window"
            | "applying"
            | "installing"
            | "committing"
            | "finalizing"
            | "pending_reboot"
            | "pending-reboot"
            | "rebooting"
            | "complete"
    )
}

fn is_ota_terminal_error_stage(stage: &str) -> bool {
    matches!(
        stage,
        "rolled_back"
            | "rollback"
            | "failed"
            | "failure"
            | "error"
            | "aborted"
            | "cancelled"
            | "insufficient_space"
            | "insufficient_storage"
            | "insufficient-space"
            | "out_of_space"
            | "no_space"
            | "disk_full"
            | "storage_full"
    )
}

fn is_ota_out_of_space_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("no space")
        || normalized.contains("out of space")
        || normalized.contains("insufficient space")
        || normalized.contains("insufficient storage")
        || normalized.contains("insufficient_storage")
        || normalized.contains("storage_full")
        || normalized.contains("not enough space")
        || normalized.contains("disk full")
        || normalized.contains("enospc")
}

fn ota_poll_error_indicates_offline(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("connection refused")
        || normalized.contains("connection reset")
        || normalized.contains("connection aborted")
        || normalized.contains("timed out")
        || normalized.contains("timeout")
        || normalized.contains("no route to host")
        || normalized.contains("network is unreachable")
        || normalized.contains("connection closed before message completed")
        || normalized.contains("failed to connect")
        || normalized.contains("dns error")
        || normalized.contains("name or service not known")
        || normalized.contains("temporary failure in name resolution")
        || normalized.contains("http 502")
        || normalized.contains("http 503")
        || normalized.contains("http 504")
}

fn ota_stage_progress_hint(stage: &str) -> Option<f64> {
    match stage {
        "downloading" => Some(15.0),
        "verifying" => Some(45.0),
        "awaiting_window" => Some(65.0),
        "applying" | "installing" | "committing" | "finalizing" => Some(85.0),
        "pending_reboot" | "pending-reboot" => Some(95.0),
        "rebooting" | "complete" => Some(100.0),
        "rolled_back" | "failed" | "failure" | "error" => Some(0.0),
        _ => None,
    }
}

fn ota_apply_stage_progress_hint(stage: &str) -> Option<f64> {
    match stage {
        "downloading" => Some(15.0),
        "verifying" => Some(35.0),
        "awaiting_window" => Some(55.0),
        "applying" | "installing" => Some(75.0),
        "committing" => Some(88.0),
        "finalizing" => Some(94.0),
        "pending_reboot" | "pending-reboot" => Some(97.0),
        "rebooting" | "complete" => Some(100.0),
        _ => None,
    }
}

fn is_device_runtime_online(client: &reqwest::blocking::Client, target_ip: &str) -> bool {
    let probes = [
        format!("http://{target_ip}:5801/v1/health"),
        format!("http://{target_ip}:5800/v1/health"),
        format!("http://{target_ip}:80/v1/health"),
        format!("http://{target_ip}:5801/v1/device/os"),
        format!("http://{target_ip}:5800/v1/device/os"),
        format!("http://{target_ip}:80/v1/device/os"),
    ];
    probes.iter().any(|url| {
        client
            .get(url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    })
}

fn emit_ota_step_progress(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    step: &str,
    status: &str,
    message: impl Into<String>,
    progress_percent: Option<f64>,
) {
    emit_ota_step_progress_with_bytes(
        app,
        run_id_ref,
        step,
        status,
        message,
        progress_percent,
        None,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_ota_step_progress_with_bytes(
    app: &tauri::AppHandle,
    run_id_ref: Option<&str>,
    step: &str,
    status: &str,
    message: impl Into<String>,
    progress_percent: Option<f64>,
    bytes_written: Option<u64>,
    bytes_total: Option<u64>,
) {
    let payload = UpdaterProgressEvent {
        run_id: run_id_ref.map(str::to_string),
        mode: "ota".to_string(),
        step: step.to_string(),
        status: status.to_string(),
        message: message.into(),
        timestamp_epoch_ms: epoch_ms(),
        stdout: None,
        stderr: None,
        exit_code: None,
        duration_ms: None,
        image_path: None,
        target_path: None,
        progress_percent,
        bytes_written,
        bytes_total,
    };
    let _ = app.emit(UPDATER_PROGRESS_EVENT, payload);
}
