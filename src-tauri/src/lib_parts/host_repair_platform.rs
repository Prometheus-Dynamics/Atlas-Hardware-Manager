fn run_host_setup_repair_internal() -> Result<HostSetupRepairResult, String> {
    let mut attempted_actions = Vec::new();
    let mut applied_actions = Vec::new();
    let mut warnings = Vec::new();

    attempted_actions.push("Resolve missing rpiboot bundle and boot files".to_string());
    match seed_user_tool_override() {
        Ok(true) => {
            applied_actions.push("Seeded user-level rpiboot tool override.".to_string());
        }
        Ok(false) => {
            warnings.push(
                "No repairable rpiboot source was found. Bundle rpiboot during build.".to_string(),
            );
        }
        Err(error) => {
            warnings.push(format!("rpiboot repair failed: {error}"));
        }
    }

    let status = build_host_setup_status();
    Ok(HostSetupRepairResult {
        status,
        attempted_actions,
        applied_actions,
        warnings,
    })
}

fn relaunch_elevated_internal() -> Result<OperationResult, String> {
    let started = Instant::now();
    let current_exe = env::current_exe()
        .map_err(|error| format!("Unable to resolve current executable path: {error}"))?;
    let exe_string = path_to_string(&current_exe);

    if cfg!(target_os = "windows") {
        if is_windows_elevated().unwrap_or(false) {
            return Ok(OperationResult {
                success: true,
                exit_code: Some(0),
                duration_ms: started.elapsed().as_millis() as u64,
                stdout: String::new(),
                stderr: String::new(),
                message: "App is already running as Administrator.".to_string(),
                timed_out: false,
            });
        }

        let powershell = resolve_required_tool("powershell")
            .or_else(|_| resolve_required_tool("pwsh"))
            .map_err(|_| {
                "PowerShell was not found, so the app cannot relaunch elevated.".to_string()
            })?;
        let script = format!(
            "Start-Process -FilePath '{}' -Verb RunAs",
            escape_powershell_single_quoted(&exe_string)
        );
        let output = Command::new(powershell)
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map_err(|error| format!("Failed to launch elevated process: {error}"))?;

        return Ok(OperationResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            duration_ms: started.elapsed().as_millis() as u64,
            stdout: trim_output(&output.stdout),
            stderr: trim_output(&output.stderr),
            message: if output.status.success() {
                "Elevated relaunch requested. Approve the UAC prompt and switch to the new window."
                    .to_string()
            } else {
                "Failed to request elevated relaunch. Verify UAC policy and retry.".to_string()
            },
            timed_out: false,
        });
    }

    if cfg!(target_os = "macos") {
        if is_unix_root() {
            return Ok(OperationResult {
                success: true,
                exit_code: Some(0),
                duration_ms: started.elapsed().as_millis() as u64,
                stdout: String::new(),
                stderr: String::new(),
                message: "App is already running with root privileges.".to_string(),
                timed_out: false,
            });
        }

        let osascript = resolve_required_tool("osascript").map_err(|_| {
            "osascript was not found, so elevated relaunch is unavailable.".to_string()
        })?;
        let escaped_exe = escape_applescript_string(&exe_string);
        let script = format!(
            "do shell script (quoted form of \"{escaped_exe}\" & \" >/dev/null 2>&1 &\") with administrator privileges"
        );
        let output = Command::new(osascript)
            .args(["-e", &script])
            .output()
            .map_err(|error| format!("Failed to launch elevated process via osascript: {error}"))?;

        return Ok(OperationResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            duration_ms: started.elapsed().as_millis() as u64,
            stdout: trim_output(&output.stdout),
            stderr: trim_output(&output.stderr),
            message: if output.status.success() {
                "Elevated relaunch requested. Approve the macOS admin prompt and use the new app window."
                    .to_string()
            } else {
                "Failed to request elevated relaunch. Verify admin prompt permissions and retry."
                    .to_string()
            },
            timed_out: false,
        });
    }

    if cfg!(target_os = "linux") {
        if is_unix_root() {
            return Ok(OperationResult {
                success: true,
                exit_code: Some(0),
                duration_ms: started.elapsed().as_millis() as u64,
                stdout: String::new(),
                stderr: String::new(),
                message: "App is already running as root.".to_string(),
                timed_out: false,
            });
        }

        let output = if let Some(pkexec_path) = find_in_path("pkexec") {
            Command::new(pkexec_path)
                .arg(&current_exe)
                .output()
                .map_err(|error| format!("Failed to launch pkexec relaunch: {error}"))?
        } else if let Some(sudo_path) = find_in_path("sudo") {
            Command::new(sudo_path)
                .args(["-E"])
                .arg(&current_exe)
                .output()
                .map_err(|error| format!("Failed to launch sudo relaunch: {error}"))?
        } else {
            return Err(
                "No elevation helper found (`pkexec`/`sudo`). Relaunch the app with root privileges."
                    .to_string(),
            );
        };

        return Ok(OperationResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            duration_ms: started.elapsed().as_millis() as u64,
            stdout: trim_output(&output.stdout),
            stderr: trim_output(&output.stderr),
            message: if output.status.success() {
                "Elevated relaunch requested. Use the new root instance and close this one."
                    .to_string()
            } else {
                "Failed to request elevated relaunch.".to_string()
            },
            timed_out: false,
        });
    }

    Err("Elevated relaunch is not supported on this platform.".to_string())
}

fn request_elevation_prompt(operation: &str) -> Result<String, String> {
    let result = relaunch_elevated_internal()?;
    if result.success {
        Ok(result.message)
    } else {
        Err(format!(
            "{operation} requires elevated permissions, and the OS prompt failed: {}",
            result.message
        ))
    }
}

fn seed_user_tool_override() -> Result<bool, String> {
    let user_root = user_tool_override_root()
        .ok_or_else(|| "Unable to resolve user data directory for tool overrides.".to_string())?;
    let platform_dir = user_root.join(format!("{}-{}", platform_tag(), arch_tag()));
    fs::create_dir_all(&platform_dir)
        .map_err(|error| format!("Failed to create user tool directory: {error}"))?;

    let mut source_rpiboot: Option<PathBuf> = None;
    let mut source_boot_dir: Option<PathBuf> = None;

    if let Some(tool) = resolve_tool("rpiboot") {
        source_boot_dir = resolve_rpiboot_boot_dir(&tool.path);
        source_rpiboot = Some(tool.path);
    }
    if source_rpiboot.is_none() {
        let candidates = ["rpiboot.exe", "rpiboot"];
        for candidate in candidates {
            if let Some(path) = find_in_path(candidate) {
                source_boot_dir = resolve_rpiboot_boot_dir(&path);
                source_rpiboot = Some(path);
                break;
            }
        }
    }

    let Some(source_binary) = source_rpiboot else {
        return Ok(false);
    };
    let binary_name = if cfg!(target_os = "windows") {
        "rpiboot.exe"
    } else {
        "rpiboot"
    };
    let target_binary = platform_dir.join(binary_name);
    fs::copy(&source_binary, &target_binary)
        .map_err(|error| format!("Failed to copy rpiboot into user tool directory: {error}"))?;
    ensure_executable_if_needed(&target_binary);

    if let Some(boot_dir) = source_boot_dir {
        let target_boot_dir = platform_dir.join("mass-storage-gadget64");
        if target_boot_dir.exists() {
            fs::remove_dir_all(&target_boot_dir)
                .map_err(|error| format!("Failed to replace boot file directory: {error}"))?;
        }
        copy_directory_recursive(&boot_dir, &target_boot_dir)
            .map_err(|error| format!("Failed to copy boot file directory: {error}"))?;
    }

    Ok(true)
}

fn user_tool_override_root() -> Option<PathBuf> {
    if let Ok(path) = env::var("ATLAS_TOOL_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    if cfg!(target_os = "windows") {
        let local_app_data = env::var("LOCALAPPDATA").ok()?;
        return Some(
            Path::new(&local_app_data)
                .join("Atlas Hardware Manager")
                .join("tools"),
        );
    }

    if cfg!(target_os = "macos") {
        let home = env::var("HOME").ok()?;
        return Some(
            Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("Atlas Hardware Manager")
                .join("tools"),
        );
    }

    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data_home.trim();
        if !trimmed.is_empty() {
            return Some(
                Path::new(trimmed)
                    .join("atlas-hardware-manager")
                    .join("tools"),
            );
        }
    }
    let home = env::var("HOME").ok()?;
    Some(
        Path::new(&home)
            .join(".local")
            .join("share")
            .join("atlas-hardware-manager")
            .join("tools"),
    )
}

fn ensure_executable_if_needed(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

fn copy_directory_recursive(source: &Path, target: &Path) -> Result<(), io::Error> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory_recursive(&source_path, &target_path)?;
        } else if source_path.is_file() {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn linux_dependency_hint(command: &str) -> String {
    match command {
        "ip" => "Install iproute2 (Debian/Ubuntu: `sudo apt install iproute2`).".to_string(),
        "lsblk" => "Install util-linux (Debian/Ubuntu: `sudo apt install util-linux`).".to_string(),
        "dd" | "sync" => {
            "Install coreutils (Debian/Ubuntu: `sudo apt install coreutils`).".to_string()
        }
        "xz" => "Install xz-utils (Debian/Ubuntu: `sudo apt install xz-utils`).".to_string(),
        _ => format!("Install `{command}` and relaunch Atlas Hardware Manager."),
    }
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_process_elevated() -> bool {
    if cfg!(target_os = "windows") {
        return is_windows_elevated().unwrap_or(false);
    }
    if cfg!(unix) {
        return is_unix_root();
    }
    false
}

fn is_unix_root() -> bool {
    if !cfg!(unix) {
        return false;
    }

    let Some(id_path) = find_in_path("id") else {
        return false;
    };
    let Ok(output) = Command::new(id_path).arg("-u").output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).trim() == "0"
}

fn is_windows_elevated() -> Option<bool> {
    if !cfg!(target_os = "windows") {
        return None;
    }

    let powershell_path = find_in_path("powershell").or_else(|| find_in_path("pwsh"))?;
    let script = "(New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)";
    let output = Command::new(powershell_path)
        .args(["-NoProfile", "-Command", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    Some(value == "true")
}

fn path_to_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn photonvision_archive_log_cache(
) -> &'static Mutex<HashMap<String, PhotonvisionArchiveLogCacheEntry>> {
    PHOTONVISION_ARCHIVE_LOG_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn build_log_probe_targets() -> Vec<(u16, &'static str)> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();

    let mut add_target = |port: u16, path: &'static str| {
        let key = format!("{port}:{path}");
        if seen.insert(key) {
            targets.push((port, path));
        }
    };

    for path in HELIOS_LOG_API_PATHS {
        add_target(80, path);
        add_target(5800, path);
        if path.starts_with("/v1/") {
            add_target(5801, path);
        }
    }

    targets
}
