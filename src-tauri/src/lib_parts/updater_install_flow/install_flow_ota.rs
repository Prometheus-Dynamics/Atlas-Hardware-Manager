use super::*;

#[cfg(target_os = "linux")]
#[path = "install_flow_ota/usb_recovery_client.rs"]
mod usb_recovery_client;
#[cfg(target_os = "linux")]
pub(crate) use usb_recovery_client::*;

pub(crate) fn install_helios_os_via_ota(
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

pub(crate) fn install_helios_os_via_usb_recovery_ota(
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
