use super::*;

pub(crate) fn parse_ota_runtime_progress_value(value: &Value, key: &str) -> Option<f64> {
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

pub(crate) fn parse_ota_runtime_number_text(value: &str) -> Option<f64> {
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

pub(crate) fn normalize_ota_stage_name(stage: String) -> String {
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

    #[test]
    fn ota_poll_error_detects_reqwest_send_failures() {
        assert!(ota_poll_error_indicates_offline(
            "http://172.31.250.1:5801/v1/ota/state: error sending request for url (http://172.31.250.1:5801/v1/ota/state)"
        ));
    }

    #[test]
    fn ota_monitor_treats_post_apply_poll_errors_as_expected_offline() {
        assert!(ota_monitor_poll_error_should_be_treated_as_offline(
            "unclassified transport failure",
            true,
            false
        ));
        assert!(ota_monitor_poll_error_should_be_treated_as_offline(
            "another transport failure",
            false,
            true
        ));
        assert!(!ota_monitor_poll_error_should_be_treated_as_offline(
            "unexpected parser error",
            false,
            false
        ));
    }
}

pub(crate) fn is_ota_apply_stage(stage: &str) -> bool {
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

pub(crate) fn is_ota_terminal_error_stage(stage: &str) -> bool {
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

pub(crate) fn is_ota_out_of_space_error(message: &str) -> bool {
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

pub(crate) fn ota_poll_error_indicates_offline(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("connection refused")
        || normalized.contains("connection reset")
        || normalized.contains("connection aborted")
        || normalized.contains("error sending request for url")
        || normalized.contains("request failed")
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

pub(crate) fn ota_monitor_poll_error_should_be_treated_as_offline(
    message: &str,
    saw_apply_related_stage: bool,
    saw_reboot_stage: bool,
) -> bool {
    saw_apply_related_stage || saw_reboot_stage || ota_poll_error_indicates_offline(message)
}

pub(crate) fn ota_stage_progress_hint(stage: &str) -> Option<f64> {
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

pub(crate) fn ota_apply_stage_progress_hint(stage: &str) -> Option<f64> {
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

pub(crate) fn is_device_runtime_online(client: &reqwest::blocking::Client, target_ip: &str) -> bool {
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

pub(crate) fn emit_ota_step_progress(
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
pub(crate) fn emit_ota_step_progress_with_bytes(
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
        mode: UpdaterMode::Ota,
        step: step.into(),
        status: status.into(),
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
    emit_updater_event_payload(app, payload);
}
