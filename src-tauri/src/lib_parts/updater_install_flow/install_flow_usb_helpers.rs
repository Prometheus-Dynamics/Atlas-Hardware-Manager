use super::*;

pub(crate) fn usb_recovery_response_id_matches(response_id: Option<&Value>, request_id: u64) -> bool {
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
pub(crate) fn ensure_no_active_usb_ota_transfer(
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
pub(crate) fn begin_usb_ota_transfer_with_stale_retry(
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
pub(crate) fn abort_active_usb_ota_transfer(
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
pub(crate) fn usb_ota_status_in_progress(status: &Value) -> bool {
    status
        .get("ota")
        .and_then(Value::as_object)
        .and_then(|ota| ota.get("in_progress"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
pub(crate) fn usb_ota_status_active_transfer_id(status: &Value) -> Option<String> {
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
pub(crate) fn usb_ota_begin_failed_due_to_active_transfer(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("ota.begin")
        && (normalized.contains("already active")
            || normalized.contains("active ota transfer")
            || normalized.contains("bad_state"))
}

#[cfg(target_os = "linux")]
pub(crate) fn connect_usb_recovery_client(
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
pub(crate) fn grant_usb_recovery_serial_access_with_elevation(paths: &[PathBuf]) -> Result<String, String> {
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
pub(crate) fn run_linux_privileged_tool_command(
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
            let pkexec_args = build_pkexec_shell_wrapper_args(program, args)?;
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
pub(crate) fn discover_usb_recovery_ports() -> Vec<PathBuf> {
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
pub(crate) fn hash_file_sha256(path: &Path) -> Result<String, String> {
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
