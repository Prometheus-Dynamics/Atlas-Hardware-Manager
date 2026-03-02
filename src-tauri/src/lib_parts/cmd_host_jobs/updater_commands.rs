use super::*;
use super::roborio_support_and_ota_probe::{
    normalize_ota_probe_targets, ota_runtime_state_is_attachable, UpdaterJobStartGuard,
};
#[path = "updater_commands/client_update_check.rs"]
mod client_update_check;

struct UpdaterWorkerManager;

impl UpdaterWorkerManager {
    fn spawn<F>(
        app_handle: tauri::AppHandle,
        run_id: Option<String>,
        mode: &str,
        panic_message: &'static str,
        clear_ota_recovery_on_exit: bool,
        worker: F,
    ) where
        F: FnOnce(&tauri::AppHandle, Option<&str>, &str) + Send + 'static,
    {
        let mode_owned = mode.to_string();
        thread::spawn(move || {
            let run_id_ref = run_id.as_deref();
            let worker_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                worker(&app_handle, run_id_ref, mode_owned.as_str());
            }));

            if worker_result.is_err() {
                emit_updater_progress(
                    &app_handle,
                    run_id_ref,
                    mode_owned.as_str(),
                    "complete",
                    "error",
                    panic_message,
                );
            }
            if clear_ota_recovery_on_exit {
                clear_ota_recovery_if_run_matches(&app_handle, run_id_ref);
            }
            finish_updater_job(run_id_ref, mode_owned.as_str());
        });
    }
}

#[tauri::command]
pub(crate) fn list_helios_release_images() -> Result<Vec<ReleaseImageOption>, String> {
    fetch_release_images_from_github()
}

#[tauri::command]
pub(crate) fn get_client_update_status() -> Result<ClientUpdateStatus, String> {
    client_update_check::get_client_update_status_internal()
}

#[tauri::command]
pub(crate) fn start_client_self_update() -> Result<ClientSelfUpdateResult, String> {
    client_update_check::start_client_self_update_internal()
}

#[tauri::command]
pub(crate) fn clear_release_image_download_cache(
) -> Result<ReleaseDownloadCacheClearResult, String> {
    clear_release_download_cache()
}

#[tauri::command]
pub(crate) fn get_host_setup_status() -> Result<HostSetupStatus, String> {
    Ok(build_host_setup_status())
}

#[tauri::command]
pub(crate) fn run_host_setup_repair() -> Result<HostSetupRepairResult, String> {
    run_host_setup_repair_internal()
}

#[tauri::command]
pub(crate) fn relaunch_elevated() -> Result<OperationResult, String> {
    relaunch_elevated_internal()
}

#[tauri::command]
pub(crate) fn probe_existing_device_ota_update(
    request: ProbeExistingOtaUpdateRequest,
) -> Result<Option<ExistingOtaUpdateProbeResult>, String> {
    let targets = normalize_ota_probe_targets(&request.target_ip_addresses);
    if targets.is_empty() {
        return Ok(None);
    }

    let timeout_ms = request.timeout_ms.unwrap_or(1500).clamp(400, 5000);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|error| format!("Unable to build OTA probe HTTP client: {error}"))?;

    for target_ip in targets {
        let api_bases = ota_api_base_urls(&target_ip);
        if api_bases.is_empty() {
            continue;
        }

        let state = match fetch_ota_runtime_state(&client, &api_bases) {
            Ok(state) => state,
            Err(_) => continue,
        };
        if !ota_runtime_state_is_attachable(&state) {
            continue;
        }

        let stage = state
            .stage
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let progress_percent = state.progress_percent.map(|value| value.clamp(0.0, 100.0));
        let update_id = state
            .update_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let last_error = state
            .last_error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        return Ok(Some(ExistingOtaUpdateProbeResult {
            target_ip_address: target_ip,
            update_id,
            stage,
            progress_percent,
            last_error,
        }));
    }

    Ok(None)
}

#[tauri::command]
pub(crate) fn attach_existing_ota_update(
    app: tauri::AppHandle,
    request: AttachExistingOtaUpdateRequest,
) -> Result<String, String> {
    let run_id = request.run_id.trim().to_string();
    if run_id.is_empty() {
        return Err("Missing run id for OTA attach workflow.".to_string());
    }
    let target_ip = request.target_ip_address.trim().to_string();
    if target_ip.is_empty() {
        return Err("Missing device IP for OTA attach workflow.".to_string());
    }

    let api_bases = ota_api_base_urls(&target_ip);
    if api_bases.is_empty() {
        return Err(format!(
            "Unable to build OTA API endpoints for target '{target_ip}'."
        ));
    }

    let probe_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1800))
        .build()
        .map_err(|error| format!("Unable to build OTA attach probe client: {error}"))?;
    let state = fetch_ota_runtime_state(&probe_client, &api_bases)
        .map_err(|error| format!("Unable to query OTA state on {target_ip}: {error}"))?;
    if !ota_runtime_state_is_attachable(&state) {
        let stage = state
            .stage
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("idle");
        return Err(format!(
            "Device at {target_ip} does not report an active OTA update (stage: {stage})."
        ));
    }

    let mut updater_start_guard = UpdaterJobStartGuard::begin(Some(run_id.as_str()), "ota")?;
    let timeout_seconds = request.timeout_seconds.unwrap_or(900).clamp(120, 1800);
    let expected_update_id = request
        .expected_update_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            state
                .update_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    persist_ota_recovery_start(
        &app,
        Some(run_id.as_str()),
        Some(target_ip.as_str()),
        Some(timeout_seconds),
    );
    persist_ota_recovery_update(
        &app,
        Some(run_id.as_str()),
        expected_update_id.clone(),
        Some("Attached to existing OTA update session.".to_string()),
    );

    let app_handle = app.clone();
    let target_ip_for_thread = target_ip.clone();
    let expected_update_id_for_thread = expected_update_id.clone();
    UpdaterWorkerManager::spawn(
        app_handle,
        Some(run_id.clone()),
        "ota",
        "Attached OTA monitor panicked unexpectedly.",
        true,
        move |thread_app_handle, run_id_ref, mode| {
            emit_updater_progress(
                thread_app_handle,
                run_id_ref,
                mode,
                "info",
                "info",
                format!(
                    "Attached to in-progress OTA update on {target_ip_for_thread}. Monitoring live OTA state."
                ),
            );

            let api_bases = ota_api_base_urls(&target_ip_for_thread);
            let result = wait_for_ota_reboot_and_reconnect(
                thread_app_handle,
                run_id_ref,
                &target_ip_for_thread,
                &api_bases,
                timeout_seconds,
                expected_update_id_for_thread.as_deref(),
            );
            match result {
                Ok(message) => {
                    emit_updater_progress(
                        thread_app_handle,
                        run_id_ref,
                        mode,
                        "complete",
                        "success",
                        message,
                    );
                }
                Err(error) => {
                    emit_updater_progress(
                        thread_app_handle,
                        run_id_ref,
                        mode,
                        "complete",
                        "error",
                        format!("Attached OTA monitor failed: {error}"),
                    );
                }
            }
        },
    );
    updater_start_guard.disarm();

    Ok(format!(
        "Attached to existing OTA update on {target_ip}. Monitoring started."
    ))
}

#[tauri::command]
pub(crate) fn cancel_helios_update(
    app: tauri::AppHandle,
    request: CancelUpdateRequest,
) -> Result<String, String> {
    let Some(active_session) = active_updater_session_snapshot() else {
        return Ok("No active updater operation is running.".to_string());
    };
    if !set_update_cancel_requested(request.run_id.as_deref(), request.mode.as_deref(), true) {
        return Ok(
            "Cancel request ignored because it does not match the active updater session."
                .to_string(),
        );
    }
    emit_updater_progress(
        &app,
        active_session.run_id.as_deref(),
        &active_session.mode,
        "cancel-requested",
        "info",
        "Cancel requested by user. Waiting for the current operation to stop.",
    );
    Ok("Cancel request accepted. Stopping current updater operation.".to_string())
}

#[tauri::command]
pub(crate) fn recover_ota_update_session(
    app: tauri::AppHandle,
) -> Result<Option<UpdaterRecoverySession>, String> {
    if UPDATER_JOB_ACTIVE.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let Some(state) = load_recoverable_ota_state(&app) else {
        return Ok(None);
    };
    let target_ip = state
        .target_ip_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Recovery state is missing OTA target IP address.".to_string())?
        .to_string();
    let mut updater_start_guard =
        UpdaterJobStartGuard::begin(Some(state.run_id.as_str()), "ota")?;
    let run_id = state.run_id.clone();
    let expected_update_id = state.expected_update_id.clone();
    let started_at_epoch_ms = state.started_at_epoch_ms;
    let timeout_seconds = state.monitor_timeout_seconds.unwrap_or(900).clamp(120, 1800);
    let target_ip_for_thread = target_ip.clone();
    let app_handle = app.clone();
    persist_ota_recovery_update(
        &app_handle,
        Some(run_id.as_str()),
        expected_update_id.clone(),
        Some("Resuming OTA monitor after app restart.".to_string()),
    );
    UpdaterWorkerManager::spawn(
        app_handle,
        Some(run_id.clone()),
        "ota",
        "Recovered OTA monitor panicked unexpectedly.",
        true,
        move |thread_app_handle, run_id_ref, mode| {
            emit_updater_progress(
                thread_app_handle,
                run_id_ref,
                mode,
                "info",
                "info",
                format!(
                    "Recovered OTA update session for {target_ip_for_thread}. Resuming reboot/reconnect monitor."
                ),
            );
            let api_bases = ota_api_base_urls(&target_ip_for_thread);
            let result = wait_for_ota_reboot_and_reconnect(
                thread_app_handle,
                run_id_ref,
                &target_ip_for_thread,
                &api_bases,
                timeout_seconds,
                expected_update_id.as_deref(),
            );
            match result {
                Ok(message) => {
                    emit_updater_progress(
                        thread_app_handle,
                        run_id_ref,
                        mode,
                        "complete",
                        "success",
                        message,
                    );
                }
                Err(error) => {
                    emit_updater_progress(
                        thread_app_handle,
                        run_id_ref,
                        mode,
                        "complete",
                        "error",
                        format!("Recovered OTA monitor failed: {error}"),
                    );
                }
            }
        },
    );
    updater_start_guard.disarm();

    Ok(Some(UpdaterRecoverySession {
        run_id: state.run_id,
        mode: "ota".to_string(),
        target_ip_address: Some(target_ip),
        expected_update_id: state.expected_update_id,
        started_at_epoch_ms,
        resumed_at_epoch_ms: epoch_ms(),
        note: Some("Recovered OTA updater session after restart.".to_string()),
    }))
}

#[tauri::command]
pub(crate) fn start_install_helios_os(
    app: tauri::AppHandle,
    request: ReleaseInstallRequest,
) -> Result<String, String> {
    let mode = if request.mount_only.unwrap_or(false) {
        "mount".to_string()
    } else if request.prefer_ota.unwrap_or(false) {
        "ota".to_string()
    } else {
        "flash".to_string()
    };
    let run_id = request.run_id.clone();
    let mut updater_start_guard = UpdaterJobStartGuard::begin(run_id.as_deref(), mode.as_str())?;
    if mode == "ota" {
        persist_ota_recovery_start(
            &app,
            run_id.as_deref(),
            request.target_ip_address.as_deref(),
            request.timeout_seconds,
        );
    } else {
        clear_updater_recovery_state(&app);
    }
    let app_handle = app.clone();
    let clear_ota_recovery_on_exit = mode == "ota";
    UpdaterWorkerManager::spawn(
        app_handle,
        run_id.clone(),
        mode.as_str(),
        "Updater worker panicked unexpectedly.",
        clear_ota_recovery_on_exit,
        move |thread_app_handle, run_id_ref, mode_name| {
            emit_updater_progress(
                thread_app_handle,
                run_id_ref,
                mode_name,
                "info",
                "info",
                "Updater started in background.",
            );
            let result = install_helios_os(thread_app_handle.clone(), request);
            if let Err(error) = result {
                emit_updater_progress(
                    thread_app_handle,
                    run_id_ref,
                    mode_name,
                    "complete",
                    "error",
                    format!("Update failed: {error}"),
                );
            }
        },
    );
    updater_start_guard.disarm();
    Ok("Updater started in background.".to_string())
}

#[tauri::command]
pub(crate) fn start_mount_helios_bootloader(
    app: tauri::AppHandle,
    request: MountRequest,
) -> Result<String, String> {
    clear_updater_recovery_state(&app);
    let run_id = request.run_id.clone();
    let mut updater_start_guard = UpdaterJobStartGuard::begin(run_id.as_deref(), "mount")?;
    let app_handle = app.clone();
    UpdaterWorkerManager::spawn(
        app_handle,
        run_id,
        "mount",
        "Mount workflow worker panicked unexpectedly.",
        false,
        move |thread_app_handle, run_id_ref, mode_name| {
            emit_updater_progress(
                thread_app_handle,
                run_id_ref,
                mode_name,
                "info",
                "info",
                "Mount workflow started in background.",
            );
            let result = mount_helios_bootloader(thread_app_handle.clone(), request);
            if let Err(error) = result {
                emit_updater_progress(
                    thread_app_handle,
                    run_id_ref,
                    mode_name,
                    "complete",
                    "error",
                    format!("Mount workflow failed: {error}"),
                );
            }
        },
    );
    updater_start_guard.disarm();
    Ok("Mount workflow started in background.".to_string())
}
