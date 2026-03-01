fn updater_terminal_complete_runs_slot() -> &'static Mutex<HashSet<String>> {
    static EMITTED_TERMINAL_COMPLETE_RUNS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    EMITTED_TERMINAL_COMPLETE_RUNS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn mark_terminal_complete_emitted_for_run(run_id: Option<&str>) -> bool {
    let Some(run_id) = normalize_updater_run_id(run_id) else {
        return true;
    };
    let mut emitted = updater_terminal_complete_runs_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if emitted.contains(&run_id) {
        return false;
    }
    emitted.insert(run_id);
    true
}

fn clear_terminal_complete_marker_for_run(run_id: Option<&str>) {
    let Some(run_id) = normalize_updater_run_id(run_id) else {
        return;
    };
    let mut emitted = updater_terminal_complete_runs_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    emitted.remove(&run_id);
}

fn emit_updater_event_payload(app: &tauri::AppHandle, payload: UpdaterProgressEvent) {
    if payload.step == UpdaterStep::Complete
        && !mark_terminal_complete_emitted_for_run(payload.run_id.as_deref())
    {
        return;
    }
    let _ = app.emit(UPDATER_PROGRESS_EVENT, payload);
}

fn emit_updater_progress(
    app: &tauri::AppHandle,
    run_id: Option<&str>,
    mode: &str,
    step: &str,
    status: &str,
    message: impl Into<String>,
) {
    let payload = UpdaterProgressEvent {
        run_id: run_id.map(str::to_string),
        mode: mode.into(),
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
        progress_percent: None,
        bytes_written: None,
        bytes_total: None,
    };
    emit_updater_event_payload(app, payload);
}

fn emit_operation_progress(
    app: &tauri::AppHandle,
    run_id: Option<&str>,
    mode: &str,
    step: &str,
    status: &str,
    message: impl Into<String>,
    operation: &OperationResult,
) {
    let payload = UpdaterProgressEvent {
        run_id: run_id.map(str::to_string),
        mode: mode.into(),
        step: step.into(),
        status: status.into(),
        message: message.into(),
        timestamp_epoch_ms: epoch_ms(),
        stdout: if operation.stdout.trim().is_empty() {
            None
        } else {
            Some(operation.stdout.clone())
        },
        stderr: if operation.stderr.trim().is_empty() {
            None
        } else {
            Some(operation.stderr.clone())
        },
        exit_code: operation.exit_code,
        duration_ms: Some(operation.duration_ms),
        image_path: None,
        target_path: None,
        progress_percent: None,
        bytes_written: None,
        bytes_total: None,
    };
    emit_updater_event_payload(app, payload);
}

fn emit_flash_write_progress(
    context: &FlashProgressContext,
    bytes_written: u64,
    bytes_total: Option<u64>,
) {
    let (progress_percent, message) = if let Some(total) = bytes_total {
        if total > 0 {
            let raw_percent = ((bytes_written as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
            if bytes_written >= total {
                (
                    Some(99.5),
                    format!(
                        "Image payload copied to {} ({bytes_written}/{total} bytes). Flushing device write cache.",
                        context.target_path
                    ),
                )
            } else {
                (
                    Some(raw_percent),
                    format!(
                        "Flashing {}: {:.1}% ({bytes_written}/{total} bytes).",
                        context.target_path, raw_percent
                    ),
                )
            }
        } else {
            (
                None,
                format!(
                    "Flashing {}: {bytes_written} bytes written.",
                    context.target_path
                ),
            )
        }
    } else {
        (
            None,
            format!(
                "Flashing {}: {bytes_written} bytes written.",
                context.target_path
            ),
        )
    };

    let payload = UpdaterProgressEvent {
        run_id: context.run_id.clone(),
        mode: UpdaterMode::Flash,
        step: UpdaterStep::Flash,
        status: UpdaterStatus::Running,
        message,
        timestamp_epoch_ms: epoch_ms(),
        stdout: None,
        stderr: None,
        exit_code: None,
        duration_ms: None,
        image_path: None,
        target_path: Some(context.target_path.clone()),
        progress_percent,
        bytes_written: Some(bytes_written),
        bytes_total,
    };
    emit_updater_event_payload(&context.app, payload);
}

fn emit_flash_elevation_prompt(context: Option<&FlashProgressContext>, method: &str) {
    let Some(context) = context else {
        return;
    };
    emit_updater_progress(
        &context.app,
        context.run_id.as_deref(),
        "flash",
        "flash",
        "info",
        format!("Requesting elevated permissions via {method}. Approve the prompt to continue."),
    );
}

#[cfg(test)]
mod updater_event_dedupe_tests {
    use super::*;

    fn updater_event_test_serial_lock() -> &'static Mutex<()> {
        static SERIAL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        SERIAL_LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn terminal_complete_marker_deduplicates_per_run_and_resets() {
        let _serial = updater_event_test_serial_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear_terminal_complete_marker_for_run(Some("dedupe-run"));
        assert!(mark_terminal_complete_emitted_for_run(Some("dedupe-run")));
        assert!(!mark_terminal_complete_emitted_for_run(Some("dedupe-run")));
        clear_terminal_complete_marker_for_run(Some("dedupe-run"));
        assert!(mark_terminal_complete_emitted_for_run(Some("dedupe-run")));
        clear_terminal_complete_marker_for_run(Some("dedupe-run"));
    }

    #[test]
    fn terminal_complete_marker_does_not_dedupe_missing_run_id() {
        let _serial = updater_event_test_serial_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(mark_terminal_complete_emitted_for_run(None));
        assert!(mark_terminal_complete_emitted_for_run(None));
    }
}

fn build_host_setup_status() -> HostSetupStatus {
    let mut checks = Vec::new();

    let rpiboot_resolved = resolve_tool("rpiboot");
    let rpiboot_path = rpiboot_resolved
        .as_ref()
        .map(|tool| path_to_string(&tool.path));
    let rpiboot_is_bundled = rpiboot_resolved
        .as_ref()
        .map(|tool| tool.source == ToolSource::Bundled)
        .unwrap_or(false);
    let rpiboot_ready =
        rpiboot_resolved.is_some() && (cfg!(debug_assertions) || rpiboot_is_bundled);
    checks.push(HostSetupCheck {
        id: "bundled-rpiboot".to_string(),
        label: "Bundled rpiboot".to_string(),
        required: true,
        ready: rpiboot_ready,
        detail: if rpiboot_resolved.is_none() {
            "rpiboot binary is missing.".to_string()
        } else if !cfg!(debug_assertions) && !rpiboot_is_bundled {
            "rpiboot was found on PATH, but production requires a bundled rpiboot binary."
                .to_string()
        } else {
            "rpiboot binary is available.".to_string()
        },
        detected: rpiboot_path,
        fix_hint: if rpiboot_ready {
            None
        } else {
            Some(
                "Run System Setup Repair. If still missing, rebuild installers with `bun run rpiboot:build`."
                    .to_string(),
            )
        },
    });

    let rpiboot_boot_dir = rpiboot_resolved
        .as_ref()
        .and_then(|tool| resolve_rpiboot_boot_dir(&tool.path));
    checks.push(HostSetupCheck {
        id: "rpiboot-bootfiles".to_string(),
        label: "rpiboot boot files".to_string(),
        required: true,
        ready: rpiboot_boot_dir.is_some(),
        detail: if rpiboot_boot_dir.is_some() {
            "mass-storage-gadget64 boot files are available.".to_string()
        } else {
            "mass-storage-gadget64 directory is missing, so rpiboot cannot expose a flash target."
                .to_string()
        },
        detected: rpiboot_boot_dir.as_ref().map(path_to_string),
        fix_hint: if rpiboot_boot_dir.is_some() {
            None
        } else {
            Some(
                "Run System Setup Repair or rebuild bundled rpiboot resources before release."
                    .to_string(),
            )
        },
    });

    if cfg!(target_os = "linux") {
        for command in LINUX_FLASH_DEPENDENCIES {
            let resolved = find_in_path(command);
            checks.push(HostSetupCheck {
                id: format!("cmd-{command}"),
                label: format!("Host command `{command}`"),
                required: true,
                ready: resolved.is_some(),
                detail: if resolved.is_some() {
                    "Required host utility is available.".to_string()
                } else {
                    format!("Required host utility `{command}` is missing from PATH.")
                },
                detected: resolved.as_ref().map(path_to_string),
                fix_hint: if resolved.is_some() {
                    None
                } else {
                    Some(linux_dependency_hint(command))
                },
            });
        }

        let elevated = is_unix_root() || has_passwordless_sudo();
        checks.push(HostSetupCheck {
            id: "linux-elevation".to_string(),
            label: "Privileged access".to_string(),
            required: false,
            ready: elevated,
            detail: if elevated {
                "Root session or passwordless sudo is available for privileged operations."
                    .to_string()
            } else {
                "Privileged operations may fail. Configure sudo access or run as root.".to_string()
            },
            detected: if is_unix_root() {
                Some("running-as-root".to_string())
            } else if has_passwordless_sudo() {
                Some("passwordless-sudo".to_string())
            } else {
                None
            },
            fix_hint: if elevated {
                None
            } else {
                Some("Grant sudo rights for rpiboot/flashing operations.".to_string())
            },
        });
    } else if cfg!(target_os = "windows") {
        for command in WINDOWS_RUNTIME_DEPENDENCIES {
            let resolved = find_in_path(command);
            checks.push(HostSetupCheck {
                id: format!("cmd-{command}"),
                label: format!("Host command `{command}`"),
                required: true,
                ready: resolved.is_some(),
                detail: if resolved.is_some() {
                    "Required host utility is available.".to_string()
                } else {
                    format!("Required host utility `{command}` is missing from PATH.")
                },
                detected: resolved.as_ref().map(path_to_string),
                fix_hint: if resolved.is_some() {
                    None
                } else {
                    Some(format!(
                        "Install `{command}` and relaunch Atlas Hardware Manager."
                    ))
                },
            });
        }

        let elevated = is_windows_elevated().unwrap_or(false);
        checks.push(HostSetupCheck {
            id: "windows-admin".to_string(),
            label: "Administrator privileges".to_string(),
            required: false,
            ready: elevated,
            detail: if elevated {
                "Process is running with Administrator privileges.".to_string()
            } else {
                "Not running as Administrator. rpiboot or raw disk flashing can fail.".to_string()
            },
            detected: Some(
                if elevated {
                    "administrator"
                } else {
                    "standard-user"
                }
                .to_string(),
            ),
            fix_hint: if elevated {
                None
            } else {
                Some("Right-click the app and choose 'Run as administrator'.".to_string())
            },
        });
    } else if cfg!(target_os = "macos") {
        for command in MACOS_RUNTIME_DEPENDENCIES {
            let resolved = find_in_path(command);
            checks.push(HostSetupCheck {
                id: format!("cmd-{command}"),
                label: format!("Host command `{command}`"),
                required: true,
                ready: resolved.is_some(),
                detail: if resolved.is_some() {
                    "Required host utility is available.".to_string()
                } else {
                    format!("Required host utility `{command}` is missing from PATH.")
                },
                detected: resolved.as_ref().map(path_to_string),
                fix_hint: if resolved.is_some() {
                    None
                } else {
                    Some(format!("Install `{command}` and relaunch the app."))
                },
            });
        }

        let elevated = is_unix_root();
        checks.push(HostSetupCheck {
            id: "macos-root".to_string(),
            label: "Root privileges".to_string(),
            required: false,
            ready: elevated,
            detail: if elevated {
                "Process is running with root privileges.".to_string()
            } else {
                "Not running as root. rpiboot/raw disk writes may require elevation.".to_string()
            },
            detected: Some(if elevated { "root" } else { "standard-user" }.to_string()),
            fix_hint: if elevated {
                None
            } else {
                Some(
                    "Launch Atlas Hardware Manager from an elevated shell when flashing or running rpiboot."
                        .to_string(),
                )
            },
        });
    }

    let ready = checks.iter().all(|check| !check.required || check.ready);
    HostSetupStatus {
        platform: platform_tag().to_string(),
        arch: arch_tag().to_string(),
        ready,
        generated_at_epoch_ms: epoch_ms(),
        checks,
    }
}
