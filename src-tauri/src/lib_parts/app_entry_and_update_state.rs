#[path = "app_entry_and_update_state/updater_state.rs"]
mod updater_state;
pub(crate) use updater_state::*;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Ok(resource_dir) = app.path().resource_dir() {
                let _ = APP_RESOURCE_DIR.set(resource_dir);
            }
            ensure_linux_desktop_icon_fallback();
            let icon = tauri::image::Image::new(FALLBACK_ICON_RGBA_32, 32, 32);
            for window in app.webview_windows().values() {
                let _ = window.set_icon(icon.clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            discover_helios_devices,
            discover_network_devices,
            telemetry_and_connection::status_and_logs::get_connection_status,
            telemetry_and_connection::status_and_logs::fetch_device_log,
            roborio_log_commands::list_roborio_wpilib_logs,
            roborio_log_commands::download_roborio_wpilib_logs,
            roborio_log_commands::delete_roborio_wpilib_logs,
            roborio_log_commands::download_and_delete_roborio_wpilib_logs,
            telemetry_and_connection::start_device_telemetry_stream,
            telemetry_and_connection::status_and_logs::stop_device_telemetry_stream,
            updater_commands::list_helios_release_images,
            updater_commands::get_client_update_status,
            updater_commands::start_client_self_update,
            updater_commands::clear_release_image_download_cache,
            updater_commands::get_host_setup_status,
            updater_commands::run_host_setup_repair,
            updater_commands::relaunch_elevated,
            updater_commands::probe_existing_device_ota_update,
            updater_commands::attach_existing_ota_update,
            updater_commands::cancel_helios_update,
            updater_commands::recover_ota_update_session,
            updater_commands::start_install_helios_os,
            updater_commands::start_mount_helios_bootloader,
            install_flow_main::install_helios_os,
            updater_mount_helpers::run_rpiboot,
            updater_mount_helpers::flash_helios_image
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(not(target_os = "linux"))]
fn ensure_linux_desktop_icon_fallback() {}

#[cfg(target_os = "linux")]
fn ensure_linux_desktop_icon_fallback() {
    fn write_if_changed(path: &Path, data: &[u8]) -> io::Result<bool> {
        if let Ok(existing) = fs::read(path) {
            if existing == data {
                return Ok(false);
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, data)?;
        Ok(true)
    }

    fn escape_desktop_exec_path(path: &str) -> String {
        let mut escaped = String::with_capacity(path.len());
        for ch in path.chars() {
            match ch {
                '\\' => escaped.push_str("\\\\"),
                ' ' => escaped.push_str("\\s"),
                '\t' => escaped.push_str("\\t"),
                '\n' => escaped.push_str("\\n"),
                '"' => escaped.push_str("\\\""),
                _ => escaped.push(ch),
            }
        }
        escaped
    }

    let executable = match env::current_exe() {
        Ok(path) => path,
        Err(_) => return,
    };
    let exe_display = executable.to_string_lossy();

    let home = match env::var("HOME") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return,
    };
    let local_share = PathBuf::from(home).join(".local").join("share");
    let icon_names = [
        "ca.prometheusdynamics.atlas-hardware-manager",
        "atlas-hardware-manager",
    ];
    let icon_variants = [
        ("32x32", FALLBACK_ICON_PNG_32),
        ("128x128", FALLBACK_ICON_PNG_128),
        ("256x256@2", FALLBACK_ICON_PNG_256),
    ];

    for (size_dir, icon_bytes) in icon_variants {
        let icon_dir = local_share
            .join("icons")
            .join("hicolor")
            .join(size_dir)
            .join("apps");
        for icon_name in icon_names {
            let icon_path = icon_dir.join(format!("{icon_name}.png"));
            let _ = write_if_changed(&icon_path, icon_bytes);
        }
    }
    let scalable_dir = local_share
        .join("icons")
        .join("hicolor")
        .join("scalable")
        .join("apps");
    for icon_name in icon_names {
        let icon_path = scalable_dir.join(format!("{icon_name}.svg"));
        let _ = write_if_changed(&icon_path, FALLBACK_ICON_SVG);
    }

    let escaped_exec = escape_desktop_exec_path(&exe_display);
    let startup_wm_class = "atlas-hardware-manager";
    let desktop_entries = [
        (
            "atlas-hardware-manager.desktop",
            "ca.prometheusdynamics.atlas-hardware-manager",
        ),
        (
            "ca.prometheusdynamics.atlas-hardware-manager.desktop",
            "ca.prometheusdynamics.atlas-hardware-manager",
        ),
    ];

    for (desktop_name, icon_name) in desktop_entries {
        let desktop_path = local_share.join("applications").join(desktop_name);
        let desktop_contents = format!(
            "[Desktop Entry]\nType=Application\nName=Atlas Hardware Manager\nComment=Instrument-grade Atlas hardware configuration tool\nExec={escaped_exec}\nIcon={icon_name}\nTerminal=false\nCategories=Utility;\nStartupNotify=true\nStartupWMClass={startup_wm_class}\n"
        );
        let _ = write_if_changed(&desktop_path, desktop_contents.as_bytes());
    }
}

fn resolve_workspace_path(workspace_path: Option<String>) -> String {
    if let Some(path) = workspace_path {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(path) = env::var("HELIOS_WORKSPACE_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    DEFAULT_HELIOS_WORKSPACE.to_string()
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

const UPDATER_RECOVERY_STATE_FILE: &str = "updater-recovery-state.json";

fn updater_recovery_state_path(app: &tauri::AppHandle) -> PathBuf {
    let base_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| env::temp_dir().join("atlas-hardware-manager"));
    base_dir.join(UPDATER_RECOVERY_STATE_FILE)
}

fn read_updater_recovery_state(app: &tauri::AppHandle) -> Option<PersistedUpdaterRecoveryState> {
    let path = updater_recovery_state_path(app);
    let data = fs::read(path).ok()?;
    serde_json::from_slice::<PersistedUpdaterRecoveryState>(&data).ok()
}

fn write_updater_recovery_state(app: &tauri::AppHandle, state: &PersistedUpdaterRecoveryState) {
    let path = updater_recovery_state_path(app);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(serialized) = serde_json::to_vec_pretty(state) {
        let _ = fs::write(path, serialized);
    }
}

fn clear_updater_recovery_state(app: &tauri::AppHandle) {
    let path = updater_recovery_state_path(app);
    let _ = fs::remove_file(path);
}

fn load_recoverable_ota_state(app: &tauri::AppHandle) -> Option<PersistedUpdaterRecoveryState> {
    let state = read_updater_recovery_state(app)?;
    let run_id_valid = !state.run_id.trim().is_empty();
    let target_ip_valid = state
        .target_ip_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    if state.mode != "ota" || state.status != "running" || !run_id_valid || !target_ip_valid {
        clear_updater_recovery_state(app);
        return None;
    }
    Some(state)
}

fn persist_ota_recovery_start(
    app: &tauri::AppHandle,
    run_id: Option<&str>,
    target_ip_address: Option<&str>,
    monitor_timeout_seconds: Option<u64>,
) {
    let Some(run_id) = run_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    let target_ip_address = target_ip_address
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if target_ip_address.is_none() {
        return;
    }
    let now = epoch_ms();
    let state = PersistedUpdaterRecoveryState {
        run_id,
        mode: "ota".to_string(),
        status: "running".to_string(),
        target_ip_address,
        expected_update_id: None,
        monitor_timeout_seconds: Some(monitor_timeout_seconds.unwrap_or(900).clamp(120, 1800)),
        started_at_epoch_ms: now,
        updated_at_epoch_ms: now,
        note: Some("OTA update started.".to_string()),
    };
    write_updater_recovery_state(app, &state);
}

fn persist_ota_recovery_update(
    app: &tauri::AppHandle,
    run_id: Option<&str>,
    expected_update_id: Option<String>,
    note: Option<String>,
) {
    let Some(run_id) = run_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(mut state) = read_updater_recovery_state(app) else {
        return;
    };
    if state.run_id != run_id || state.mode != "ota" || state.status != "running" {
        return;
    }
    if let Some(update_id) = expected_update_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state.expected_update_id = Some(update_id.to_string());
    }
    if let Some(text) = note {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            state.note = Some(trimmed.to_string());
        }
    }
    state.updated_at_epoch_ms = epoch_ms();
    write_updater_recovery_state(app, &state);
}

fn clear_ota_recovery_if_run_matches(app: &tauri::AppHandle, run_id: Option<&str>) {
    let Some(run_id) = run_id.map(str::trim).filter(|value| !value.is_empty()) else {
        clear_updater_recovery_state(app);
        return;
    };
    let Some(state) = read_updater_recovery_state(app) else {
        return;
    };
    if state.run_id == run_id {
        clear_updater_recovery_state(app);
    }
}
