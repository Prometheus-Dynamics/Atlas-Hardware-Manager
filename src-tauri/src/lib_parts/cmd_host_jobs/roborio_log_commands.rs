use super::*;
use super::roborio_support_and_ota_probe::{
    normalize_roborio_host, normalize_wpilib_log_selections, parse_wpilib_log_list_line,
    resolve_download_directory, run_roborio_scp_command, run_roborio_ssh_command,
    shell_quote_remote_path, unique_download_destination,
};

pub(crate) fn fetch_device_log_blocking(request: DeviceLogRequest) -> Result<DeviceLogResult, String> {
    let ip = request.ip_address.trim();
    if ip.is_empty() {
        return Err("Missing device IP address.".to_string());
    }

    let max_lines = request.max_lines.unwrap_or(500).clamp(100, 4000);
    let runtime_product = request
        .runtime_product
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if runtime_product
        .map(is_photonvision_product)
        .unwrap_or(false)
    {
        let websocket_logs = fetch_photonvision_client_logs(ip, max_lines);
        let websocket_line_count = websocket_logs
            .as_ref()
            .map(|(_, line_count, _)| *line_count)
            .unwrap_or(0);
        let archived_logs = if websocket_line_count < 8 {
            fetch_photonvision_archived_logs(ip, max_lines)
        } else {
            None
        };

        if let Some((websocket_text, _, _)) = websocket_logs.as_ref() {
            if let Some((archive_text, archive_line_count, archive_truncated, archive_source_url)) =
                archived_logs.as_ref()
            {
                let (merged_text, merged_line_count, merged_truncated) =
                    merge_log_texts(archive_text, websocket_text, max_lines);
                return Ok(DeviceLogResult {
                    success: true,
                    source_url: Some(format!(
                        "ws://{ip}:5800/websocket_data + {archive_source_url}"
                    )),
                    line_count: merged_line_count,
                    truncated: merged_truncated || *archive_truncated,
                    fetched_at_epoch_ms: epoch_ms(),
                    log_text: merged_text,
                    message: format!(
                        "PhotonVision client logs loaded (live websocket + {} archived lines).",
                        archive_line_count
                    ),
                });
            }

            let (websocket_text, line_count, truncated) = websocket_logs.unwrap();
            return Ok(DeviceLogResult {
                success: true,
                source_url: Some(format!("ws://{ip}:5800/websocket_data")),
                line_count,
                truncated,
                fetched_at_epoch_ms: epoch_ms(),
                log_text: websocket_text,
                message: "PhotonVision client logs loaded from websocket.".to_string(),
            });
        }

        if let Some((archive_text, line_count, truncated, archive_source_url)) = archived_logs {
            return Ok(DeviceLogResult {
                success: true,
                source_url: Some(archive_source_url),
                line_count,
                truncated,
                fetched_at_epoch_ms: epoch_ms(),
                log_text: archive_text,
                message: "PhotonVision client logs loaded from archived log files.".to_string(),
            });
        }

        return Ok(DeviceLogResult {
            success: false,
            source_url: Some(format!("ws://{ip}:5800/websocket_data")),
            line_count: 0,
            truncated: false,
            fetched_at_epoch_ms: epoch_ms(),
            log_text: String::new(),
            message: "No PhotonVision client logs received from websocket.".to_string(),
        });
    }

    let probe_targets = build_log_probe_targets();
    if probe_targets.is_empty() {
        return Ok(DeviceLogResult {
            success: false,
            source_url: None,
            line_count: 0,
            truncated: false,
            fetched_at_epoch_ms: epoch_ms(),
            log_text: String::new(),
            message: "No known log endpoints for this device type.".to_string(),
        });
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(3200))
        .build()
        .map_err(|error| format!("Unable to build log HTTP client: {error}"))?;

    for (port, path) in probe_targets {
        let url = format!("http://{ip}:{port}{path}");
        let response = match client
            .get(&url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .header(
                reqwest::header::ACCEPT,
                "application/json,text/plain,application/zip,*/*",
            )
            .send()
        {
            Ok(response) => response,
            Err(_) => continue,
        };

        if !response.status().is_success() {
            continue;
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let payload = match response.bytes() {
            Ok(body) => body,
            Err(_) => continue,
        };

        let Some(raw_log_text) = extract_log_text_from_response(&payload, content_type.as_deref())
        else {
            continue;
        };
        let (log_text, line_count, truncated) = constrain_log_lines(&raw_log_text, max_lines);
        if log_text.trim().is_empty() {
            continue;
        }

        return Ok(DeviceLogResult {
            success: true,
            source_url: Some(url),
            line_count,
            truncated,
            fetched_at_epoch_ms: epoch_ms(),
            log_text,
            message: "Device logs loaded.".to_string(),
        });
    }

    Ok(DeviceLogResult {
        success: false,
        source_url: None,
        line_count: 0,
        truncated: false,
        fetched_at_epoch_ms: epoch_ms(),
        log_text: String::new(),
        message: "No log endpoint responded for this device.".to_string(),
    })
}

#[tauri::command]
pub(crate) async fn list_roborio_wpilib_logs(
    request: WpilibLogListRequest,
) -> Result<WpilibLogListResult, String> {
    RoboRioLogService::list_roborio_wpilib_logs(request)
        .await
        .map_err(|error| error.to_string())
}

fn list_roborio_wpilib_logs_blocking(
    request: WpilibLogListRequest,
) -> Result<WpilibLogListResult, String> {
    let host = normalize_roborio_host(&request.ip_address)?;
    let list_output = run_roborio_ssh_command(
        &host,
        "sh -c 'for d in /home/lvuser/logs /u/logs; do [ -d \"$d\" ] || continue; for f in \"$d\"/*.wpilog; do [ -f \"$f\" ] || continue; size=$(wc -c < \"$f\" 2>/dev/null || echo 0); mtime=$(stat -c %Y \"$f\" 2>/dev/null || busybox stat -c %Y \"$f\" 2>/dev/null || echo 0); base=$(basename \"$f\"); printf \"%s|%s|%s|%s\\n\" \"$d\" \"$base\" \"$size\" \"$mtime\"; done; done'",
        Duration::from_secs(10),
    )?;
    let stdout = String::from_utf8_lossy(&list_output.stdout);
    let mut entries = stdout
        .lines()
        .filter_map(parse_wpilib_log_list_line)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| Reverse(entry.modified_epoch_ms.unwrap_or(0)));
    entries.dedup_by(|left, right| left.remote_path == right.remote_path);

    let message = if entries.is_empty() {
        "No WPILib logs found on roboRIO.".to_string()
    } else {
        format!("Found {} WPILib log file(s) on roboRIO.", entries.len())
    };

    Ok(WpilibLogListResult {
        success: true,
        fetched_at_epoch_ms: epoch_ms(),
        entries,
        message,
    })
}

#[tauri::command]
pub(crate) async fn download_roborio_wpilib_logs(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    RoboRioLogService::download_roborio_wpilib_logs(request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn delete_roborio_wpilib_logs(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    RoboRioLogService::delete_roborio_wpilib_logs(request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn download_and_delete_roborio_wpilib_logs(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    RoboRioLogService::download_and_delete_roborio_wpilib_logs(request)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) struct RoboRioLogService;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RoboRioLogError {
    #[error("{0}")]
    Operation(String),
    #[error("WPILib log worker failed: {0}")]
    Worker(String),
}

impl RoboRioLogService {
    pub(crate) async fn list_roborio_wpilib_logs(
        request: WpilibLogListRequest,
    ) -> Result<WpilibLogListResult, RoboRioLogError> {
        tauri::async_runtime::spawn_blocking(move || list_roborio_wpilib_logs_blocking(request))
            .await
            .map_err(|error| RoboRioLogError::Worker(error.to_string()))?
            .map_err(RoboRioLogError::Operation)
    }

    pub(crate) async fn download_roborio_wpilib_logs(
        request: WpilibLogsActionRequest,
    ) -> Result<WpilibLogActionResult, RoboRioLogError> {
        tauri::async_runtime::spawn_blocking(move || {
            perform_roborio_wpilib_download(
                &request.ip_address,
                &request.selected_logs,
                request.download_directory,
            )
        })
        .await
        .map_err(|error| RoboRioLogError::Worker(error.to_string()))?
        .map_err(RoboRioLogError::Operation)
    }

    pub(crate) async fn delete_roborio_wpilib_logs(
        request: WpilibLogsActionRequest,
    ) -> Result<WpilibLogActionResult, RoboRioLogError> {
        tauri::async_runtime::spawn_blocking(move || {
            perform_roborio_wpilib_delete(&request.ip_address, &request.selected_logs)
        })
        .await
        .map_err(|error| RoboRioLogError::Worker(error.to_string()))?
        .map_err(RoboRioLogError::Operation)
    }

    pub(crate) async fn download_and_delete_roborio_wpilib_logs(
        request: WpilibLogsActionRequest,
    ) -> Result<WpilibLogActionResult, RoboRioLogError> {
        tauri::async_runtime::spawn_blocking(move || {
            perform_roborio_wpilib_download_and_delete(request)
        })
        .await
        .map_err(|error| RoboRioLogError::Worker(error.to_string()))?
        .map_err(RoboRioLogError::Operation)
    }
}

fn perform_roborio_wpilib_download_and_delete(
    request: WpilibLogsActionRequest,
) -> Result<WpilibLogActionResult, String> {
    let download_result = perform_roborio_wpilib_download(
        &request.ip_address,
        &request.selected_logs,
        request.download_directory.clone(),
    )?;
    let downloaded_paths = download_result
        .results
        .iter()
        .filter(|item| item.success)
        .map(|item| item.remote_path.clone())
        .collect::<HashSet<_>>();
    let selected_for_delete = request
        .selected_logs
        .iter()
        .filter(|entry| downloaded_paths.contains(&entry.remote_path))
        .cloned()
        .collect::<Vec<_>>();
    if selected_for_delete.is_empty() {
        return Ok(download_result);
    }

    let delete_result = perform_roborio_wpilib_delete(&request.ip_address, &selected_for_delete)?;
    let mut merged_items = Vec::new();
    let mut success_count = 0usize;
    for download_item in &download_result.results {
        let delete_item = delete_result
            .results
            .iter()
            .find(|candidate| candidate.remote_path == download_item.remote_path);
        let (success, message) = if let Some(delete_item) = delete_item {
            (
                download_item.success && delete_item.success,
                format!("{} {}", download_item.message, delete_item.message),
            )
        } else {
            (download_item.success, download_item.message.clone())
        };
        if success {
            success_count += 1;
        }
        merged_items.push(WpilibLogActionItemResult {
            file_name: download_item.file_name.clone(),
            remote_path: download_item.remote_path.clone(),
            local_path: download_item.local_path.clone(),
            success,
            message,
        });
    }

    let processed_count = merged_items.len();
    let failed_count = processed_count.saturating_sub(success_count);
    Ok(WpilibLogActionResult {
        success: failed_count == 0,
        processed_count,
        success_count,
        failed_count,
        completed_at_epoch_ms: epoch_ms(),
        download_directory: download_result.download_directory.clone(),
        results: merged_items,
        message: if failed_count == 0 {
            format!("Downloaded and deleted {} WPILib log file(s).", processed_count)
        } else {
            format!(
                "Downloaded and deleted with {} failure(s). Check per-file results.",
                failed_count
            )
        },
    })
}

fn perform_roborio_wpilib_download(
    ip_address: &str,
    selected_logs: &[WpilibLogSelection],
    download_directory: Option<String>,
) -> Result<WpilibLogActionResult, String> {
    let host = normalize_roborio_host(ip_address)?;
    let entries = normalize_wpilib_log_selections(selected_logs)?;
    if entries.is_empty() {
        return Err("No WPILib log files were selected.".to_string());
    }
    let download_root = resolve_download_directory(download_directory)?;
    fs::create_dir_all(&download_root).map_err(|error| {
        format!(
            "Unable to create download directory '{}': {error}",
            download_root.to_string_lossy()
        )
    })?;

    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let local_path = unique_download_destination(&download_root, &entry.file_name);
        let local_path_text = local_path.to_string_lossy().to_string();
        match run_roborio_scp_command(
            &host,
            &entry.remote_path,
            &local_path_text,
            Duration::from_secs(12),
        ) {
            Ok(()) => {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: Some(local_path_text),
                    success: true,
                    message: "Downloaded.".to_string(),
                });
            }
            Err(error) => {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: None,
                    success: false,
                    message: error,
                });
            }
        }
    }

    let success_count = results.iter().filter(|item| item.success).count();
    let processed_count = results.len();
    let failed_count = processed_count.saturating_sub(success_count);
    Ok(WpilibLogActionResult {
        success: failed_count == 0,
        processed_count,
        success_count,
        failed_count,
        completed_at_epoch_ms: epoch_ms(),
        download_directory: Some(download_root.to_string_lossy().to_string()),
        results,
        message: if failed_count == 0 {
            format!("Downloaded {} WPILib log file(s).", processed_count)
        } else {
            format!(
                "Downloaded {} file(s) with {} failure(s).",
                success_count, failed_count
            )
        },
    })
}

fn perform_roborio_wpilib_delete(
    ip_address: &str,
    selected_logs: &[WpilibLogSelection],
) -> Result<WpilibLogActionResult, String> {
    let host = normalize_roborio_host(ip_address)?;
    let entries = normalize_wpilib_log_selections(selected_logs)?;
    if entries.is_empty() {
        return Err("No WPILib log files were selected.".to_string());
    }

    let mut quoted_paths = Vec::with_capacity(entries.len());
    for entry in &entries {
        quoted_paths.push(shell_quote_remote_path(&entry.remote_path));
    }
    let delete_command = format!("sh -c 'rm -f -- {}'", quoted_paths.join(" "));
    let delete_result = run_roborio_ssh_command(&host, &delete_command, Duration::from_secs(10));

    let mut results = Vec::with_capacity(entries.len());
    match delete_result {
        Ok(_) => {
            for entry in entries {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: None,
                    success: true,
                    message: "Deleted.".to_string(),
                });
            }
        }
        Err(error) => {
            for entry in entries {
                results.push(WpilibLogActionItemResult {
                    file_name: entry.file_name,
                    remote_path: entry.remote_path,
                    local_path: None,
                    success: false,
                    message: format!("Delete failed: {error}"),
                });
            }
        }
    }

    let success_count = results.iter().filter(|item| item.success).count();
    let processed_count = results.len();
    let failed_count = processed_count.saturating_sub(success_count);
    Ok(WpilibLogActionResult {
        success: failed_count == 0,
        processed_count,
        success_count,
        failed_count,
        completed_at_epoch_ms: epoch_ms(),
        download_directory: None,
        results,
        message: if failed_count == 0 {
            format!("Deleted {} WPILib log file(s).", processed_count)
        } else {
            format!("Delete failed for {} file(s).", failed_count)
        },
    })
}
