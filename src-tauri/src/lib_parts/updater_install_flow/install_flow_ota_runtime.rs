use super::*;

pub(crate) fn wait_for_ota_reboot_and_reconnect(
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
    let mut last_http_poll_attempt = Instant::now() - Duration::from_secs(15);
    let mut last_ws_state: Option<OtaRuntimeState> = None;
    let mut ota_ws_monitor = match OtaWebsocketMonitor::connect(target_ip) {
        Ok(monitor) => {
            emit_ota_step_progress(
                app,
                run_id_ref,
                "ota-monitor",
                "info",
                format!(
                    "Connected OTA websocket monitor at {}. Using websocket-first runtime updates.",
                    monitor.source_url()
                ),
                None,
            );
            Some(monitor)
        }
        Err(error) => {
            emit_ota_step_progress(
                app,
                run_id_ref,
                "ota-monitor",
                "info",
                format!(
                    "OTA websocket monitor unavailable ({error}). Falling back to HTTP OTA state polling."
                ),
                None,
            );
            None
        }
    };
    let mut last_ws_connect_attempt = Instant::now();

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

        if ota_ws_monitor.is_none() && last_ws_connect_attempt.elapsed() >= Duration::from_secs(12)
        {
            last_ws_connect_attempt = Instant::now();
            if let Ok(monitor) = OtaWebsocketMonitor::connect(target_ip) {
                emit_ota_step_progress(
                    app,
                    run_id_ref,
                    "ota-monitor",
                    "info",
                    format!(
                        "OTA websocket monitor reconnected at {}.",
                        monitor.source_url()
                    ),
                    None,
                );
                ota_ws_monitor = Some(monitor);
            }
        }

        let mut drop_ws_monitor = false;
        let state_result = if let Some(ws_monitor) = ota_ws_monitor.as_mut() {
            match ws_monitor.poll_runtime_state() {
                Ok(Some(state)) => {
                    last_ws_state = Some(state.clone());
                    Ok(state)
                }
                Ok(None) => {
                    if last_http_poll_attempt.elapsed() >= Duration::from_secs(12)
                        || last_ws_state.is_none()
                    {
                        last_http_poll_attempt = Instant::now();
                        fetch_ota_runtime_state(&poll_client, api_bases)
                    } else {
                        match last_ws_state.clone() {
                            Some(state) => Ok(state),
                            None => {
                                last_http_poll_attempt = Instant::now();
                                fetch_ota_runtime_state(&poll_client, api_bases)
                            }
                        }
                    }
                }
                Err(error) => {
                    emit_ota_step_progress(
                        app,
                        run_id_ref,
                        "ota-monitor",
                        "info",
                        format!(
                            "OTA websocket monitor lost ({error}). Continuing with HTTP OTA polling."
                        ),
                        None,
                    );
                    drop_ws_monitor = true;
                    last_http_poll_attempt = Instant::now();
                    fetch_ota_runtime_state(&poll_client, api_bases)
                }
            }
        } else {
            last_http_poll_attempt = Instant::now();
            fetch_ota_runtime_state(&poll_client, api_bases)
        };
        if drop_ws_monitor {
            ota_ws_monitor = None;
            last_ws_connect_attempt = Instant::now() - Duration::from_secs(10);
            last_ws_state = None;
        }

        match state_result {
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
                let treat_as_offline = ota_monitor_poll_error_should_be_treated_as_offline(
                    &error,
                    saw_apply_related_stage,
                    saw_reboot_stage,
                );
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
                    if !saw_offline_after_apply {
                        saw_offline_after_apply = true;
                        emit_ota_step_progress(
                            app,
                            run_id_ref,
                            "ota-monitor",
                            "running",
                            format!(
                                "OTA state endpoint became unreachable after apply ({error}). Treating this as expected reboot transition."
                            ),
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
                } else if !treat_as_offline && last_wait_notice.elapsed() >= Duration::from_secs(20)
                {
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
