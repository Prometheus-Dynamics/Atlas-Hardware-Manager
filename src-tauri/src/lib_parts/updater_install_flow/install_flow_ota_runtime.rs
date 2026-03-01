use super::*;

pub(crate) fn ota_api_base_urls(target_ip: &str) -> Vec<String> {
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

pub(crate) fn fetch_ota_runtime_state(
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

pub(crate) fn parse_ota_runtime_state(raw: Option<&Value>) -> OtaRuntimeState {
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

pub(crate) fn collect_ota_runtime_state_sources(
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

