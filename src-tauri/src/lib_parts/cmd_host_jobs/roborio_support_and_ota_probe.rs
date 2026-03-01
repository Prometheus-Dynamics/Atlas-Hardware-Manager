use super::*;

pub(crate) fn run_roborio_ssh_command(
    host: &str,
    remote_command: &str,
    timeout: Duration,
) -> Result<Output, String> {
    let target = format!("lvuser@{host}");
    let mut last_error = "SSH command failed for roboRIO.".to_string();
    for option_profile in roborio_ssh_option_profiles() {
        let mut args = option_profile;
        args.push("-T".to_string());
        args.push(target.clone());
        args.push(remote_command.to_string());
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command_with_timeout("ssh", &arg_refs, timeout)?;
        if result.timed_out {
            return Err("SSH command timed out while communicating with roboRIO.".to_string());
        }
        if result.output.status.success() {
            return Ok(result.output);
        }

        let stderr = String::from_utf8_lossy(&result.output.stderr).trim().to_string();
        last_error = if stderr.is_empty() {
            "SSH command failed for roboRIO.".to_string()
        } else {
            format!("SSH command failed for roboRIO: {stderr}")
        };
        if !should_retry_roborio_ssh_with_legacy_options(&stderr) {
            break;
        }
    }

    Err(last_error)
}

pub(crate) fn normalize_roborio_host(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Missing roboRIO host/IP address.".to_string());
    }
    let valid = trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':'));
    if !valid {
        return Err("Invalid roboRIO host/IP format.".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn run_roborio_scp_command(
    host: &str,
    remote_path: &str,
    local_path: &str,
    timeout: Duration,
) -> Result<(), String> {
    let remote_spec = format!("lvuser@{host}:{remote_path}");
    let mut last_error = "Download failed.".to_string();

    for option_profile in roborio_scp_option_profiles() {
        let mut args = vec!["-q".to_string()];
        args.extend(option_profile);
        args.push(remote_spec.clone());
        args.push(local_path.to_string());
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command_with_timeout("scp", &arg_refs, timeout)?;
        if result.timed_out {
            return Err("Download timed out.".to_string());
        }
        if result.output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&result.output.stderr).trim().to_string();
        last_error = if stderr.is_empty() {
            "Download failed.".to_string()
        } else {
            format!("Download failed: {stderr}")
        };
        if !should_retry_roborio_ssh_with_legacy_options(&stderr) {
            break;
        }
    }

    Err(last_error)
}

fn roborio_ssh_option_profiles() -> Vec<Vec<String>> {
    let known_hosts_sink = roborio_known_hosts_sink().to_string();
    vec![
        vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "NumberOfPasswordPrompts=0".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=accept-new".to_string(),
            "-o".to_string(),
            "ConnectTimeout=4".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=2".to_string(),
            "-o".to_string(),
            "ServerAliveInterval=2".to_string(),
            "-o".to_string(),
            "ServerAliveCountMax=2".to_string(),
        ],
        vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "NumberOfPasswordPrompts=0".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            format!("UserKnownHostsFile={known_hosts_sink}"),
            "-o".to_string(),
            "ConnectTimeout=4".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=2".to_string(),
        ],
        vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "NumberOfPasswordPrompts=0".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            format!("UserKnownHostsFile={known_hosts_sink}"),
            "-o".to_string(),
            "HostKeyAlgorithms=+ssh-rsa".to_string(),
            "-o".to_string(),
            "PubkeyAcceptedAlgorithms=+ssh-rsa".to_string(),
            "-o".to_string(),
            "ConnectTimeout=4".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=2".to_string(),
        ],
    ]
}

fn roborio_scp_option_profiles() -> Vec<Vec<String>> {
    roborio_ssh_option_profiles()
}

fn roborio_known_hosts_sink() -> &'static str {
    if cfg!(target_os = "windows") {
        "NUL"
    } else {
        "/dev/null"
    }
}

fn should_retry_roborio_ssh_with_legacy_options(stderr: &str) -> bool {
    let normalized = stderr.to_ascii_lowercase();
    normalized.contains("host key verification failed")
        || normalized.contains("remote host identification has changed")
        || normalized.contains("no matching host key type found")
        || normalized.contains("unable to negotiate with")
        || normalized.contains("bad configuration option")
        || normalized.contains("key exchange")
}

pub(crate) fn parse_wpilib_log_list_line(line: &str) -> Option<WpilibLogEntry> {
    let mut parts = line.split('|');
    let directory = parts.next()?.trim();
    let file_name = parts.next()?.trim();
    let size_bytes = parts.next()?.trim().parse::<u64>().ok()?;
    let modified_epoch_ms = parts
        .next()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000));

    if !WPILIB_ROBORIO_LOG_DIRS.contains(&directory) || !is_safe_wpilib_log_filename(file_name) {
        return None;
    }
    let remote_path = format!("{directory}/{file_name}");
    let id = format!(
        "{}:{}",
        directory.trim_start_matches('/').replace('/', "-"),
        file_name
    );
    Some(WpilibLogEntry {
        id,
        file_name: file_name.to_string(),
        remote_path,
        size_bytes,
        modified_epoch_ms,
    })
}

pub(crate) fn normalize_wpilib_log_selections(
    selections: &[WpilibLogSelection],
) -> Result<Vec<WpilibLogSelection>, String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for selection in selections {
        let file_name = selection.file_name.trim();
        let remote_path = selection.remote_path.trim();
        if !is_safe_wpilib_log_filename(file_name) {
            continue;
        }
        let Some(clean_remote_path) = normalize_safe_wpilib_remote_path(remote_path) else {
            continue;
        };
        if !clean_remote_path.ends_with(file_name) {
            continue;
        }
        if !seen.insert(clean_remote_path.clone()) {
            continue;
        }
        normalized.push(WpilibLogSelection {
            file_name: file_name.to_string(),
            remote_path: clean_remote_path,
        });
    }
    if normalized.is_empty() {
        return Err("No valid WPILib log selections were provided.".to_string());
    }
    Ok(normalized)
}

fn normalize_safe_wpilib_remote_path(remote_path: &str) -> Option<String> {
    let trimmed = remote_path.trim();
    if trimmed.contains("..") || trimmed.contains('\\') || trimmed.contains('\0') {
        return None;
    }
    for directory in WPILIB_ROBORIO_LOG_DIRS {
        let prefix = format!("{directory}/");
        if !trimmed.starts_with(&prefix) {
            continue;
        }
        let file_name = &trimmed[prefix.len()..];
        if is_safe_wpilib_log_filename(file_name) {
            return Some(format!("{directory}/{file_name}"));
        }
    }
    None
}

fn is_safe_wpilib_log_filename(file_name: &str) -> bool {
    let trimmed = file_name.trim();
    if trimmed.is_empty() || trimmed.len() > 180 || !trimmed.ends_with(".wpilog") {
        return false;
    }
    trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

pub(crate) fn resolve_download_directory(download_directory: Option<String>) -> Result<PathBuf, String> {
    let Some(directory) = download_directory else {
        return Err("Missing download directory for WPILib logs.".to_string());
    };
    let trimmed = directory.trim();
    if trimmed.is_empty() {
        return Err("Download directory cannot be empty.".to_string());
    }
    Ok(PathBuf::from(trimmed))
}

pub(crate) fn unique_download_destination(directory: &Path, file_name: &str) -> PathBuf {
    let mut candidate = directory.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("log");
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wpilog");

    for index in 1..5000 {
        let replacement = directory.join(format!("{stem}_{index}.{extension}"));
        if !replacement.exists() {
            return replacement;
        }
        candidate = replacement;
    }

    candidate
}

pub(crate) fn shell_quote_remote_path(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}

pub(crate) fn normalize_ota_probe_targets(raw_targets: &[String]) -> Vec<String> {
    let mut targets = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for raw in raw_targets {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        targets.push(trimmed.to_string());
    }
    targets
}

pub(crate) fn ota_runtime_state_is_attachable(state: &OtaRuntimeState) -> bool {
    let stage = state
        .stage
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("idle")
        .to_ascii_lowercase();
    if stage == "idle" {
        return false;
    }
    if is_ota_terminal_error_stage(&stage) {
        return false;
    }
    if is_ota_apply_stage(&stage) {
        return true;
    }
    state
        .progress_percent
        .map(|value| value > 0.0)
        .unwrap_or(false)
        || state
            .update_id
            .as_deref()
            .map(str::trim)
            .map(|value| !value.is_empty())
            .unwrap_or(false)
}

pub(crate) struct UpdaterJobStartGuard {
    run_id: Option<String>,
    mode: String,
    armed: bool,
}

impl UpdaterJobStartGuard {
    pub(crate) fn begin(run_id: Option<&str>, mode: &str) -> Result<Self, String> {
        begin_updater_job(run_id, mode)?;
        let normalized_mode =
            normalize_updater_mode(Some(mode)).unwrap_or_else(|| "flash".to_string());
        Ok(Self {
            run_id: normalize_updater_run_id(run_id),
            mode: normalized_mode,
            armed: true,
        })
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UpdaterJobStartGuard {
    fn drop(&mut self) {
        if self.armed {
            finish_updater_job(self.run_id.as_deref(), self.mode.as_str());
        }
    }
}
