use super::*;

pub(crate) fn resolve_install_image_path(
    app: &tauri::AppHandle,
    request: &ReleaseInstallRequest,
    run_id: Option<&str>,
    mode: &str,
) -> Result<(String, String), String> {
    let release_url = request
        .release_download_url
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let local_path = request
        .local_image_path
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());

    if release_url.is_some() && local_path.is_some() {
        return Err("Choose either a release image or a local image path, not both.".to_string());
    }

    if let Some(path) = local_path {
        let image_path = Path::new(path);
        if !image_path.is_file() {
            return Err(format!("Local image path does not exist: {path}"));
        }
        return Ok((path.to_string(), "local".to_string()));
    }

    if let Some(url) = release_url {
        if !is_supported_release_download_url(url) {
            return Err("Only GitHub release download URLs are allowed.".to_string());
        }
        let downloaded = download_release_asset(url, app, run_id, mode)?;
        return Ok((downloaded, "release".to_string()));
    }

    Err("Select a release image or provide a local image path.".to_string())
}

fn is_supported_release_download_url(url: &str) -> bool {
    url.starts_with("https://github.com/")
        || url.starts_with("https://objects.githubusercontent.com/")
        || url.starts_with("https://github-releases.githubusercontent.com/")
}

fn release_download_cache_dir() -> PathBuf {
    env::temp_dir().join("atlas-hardware-manager").join("downloads")
}

fn sanitize_release_file_name(file_name: &str) -> String {
    file_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
}

fn release_download_cache_key(url: &str) -> String {
    use sha2::Digest;

    let mut hasher = sha2::Sha256::new();
    hasher.update(url.as_bytes());
    let digest = hasher.finalize();
    let mut key = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

fn cache_file_name_for_release_url(url: &str) -> String {
    let file_name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("helios-release.img");
    let safe_file_name = sanitize_release_file_name(file_name);
    let cache_key = release_download_cache_key(url);
    format!("{cache_key}-{safe_file_name}")
}

fn collect_path_stats(path: &Path) -> (u64, u64) {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return (0, 0),
    };

    if metadata.is_file() {
        return (1, metadata.len());
    }

    if !metadata.is_dir() {
        return (0, 0);
    }

    let mut files = 0u64;
    let mut bytes = 0u64;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return (0, 0),
    };
    for entry in entries.flatten() {
        let (nested_files, nested_bytes) = collect_path_stats(&entry.path());
        files += nested_files;
        bytes += nested_bytes;
    }

    (files, bytes)
}

pub(crate) fn clear_release_download_cache() -> Result<ReleaseDownloadCacheClearResult, String> {
    let cache_dir = release_download_cache_dir();
    if !cache_dir.exists() {
        return Ok(ReleaseDownloadCacheClearResult {
            removed_files: 0,
            removed_bytes: 0,
            cache_directory: path_to_string(cache_dir),
            message: "Release image cache is already empty.".to_string(),
        });
    }

    let mut removed_files = 0u64;
    let mut removed_bytes = 0u64;
    let entries = fs::read_dir(&cache_dir)
        .map_err(|error| format!("Unable to inspect release image cache directory: {error}"))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("Unable to read release image cache entry: {error}"))?;
        let path = entry.path();
        let (files, bytes) = collect_path_stats(&path);
        removed_files += files;
        removed_bytes += bytes;

        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "Unable to inspect cache entry '{}': {error}",
                path_to_string(&path)
            )
        })?;
        if metadata.is_dir() {
            fs::remove_dir_all(&path).map_err(|error| {
                format!(
                    "Unable to remove cache directory '{}': {error}",
                    path_to_string(&path)
                )
            })?;
        } else {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "Unable to remove cached file '{}': {error}",
                    path_to_string(&path)
                )
            })?;
        }
    }

    let message = if removed_files == 0 {
        "Release image cache is already empty.".to_string()
    } else {
        format!("Cleared {removed_files} cached release file(s) ({removed_bytes} bytes).")
    };

    Ok(ReleaseDownloadCacheClearResult {
        removed_files,
        removed_bytes,
        cache_directory: path_to_string(cache_dir),
        message,
    })
}

fn download_release_asset(
    url: &str,
    app: &tauri::AppHandle,
    run_id: Option<&str>,
    mode: &str,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("Unable to build HTTP client: {error}"))?;

    let mut response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
        .send()
        .map_err(|error| format!("Failed to download release image: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Release image download failed with status {}",
            response.status()
        ));
    }
    let total_bytes = response.content_length();

    let base_dir = release_download_cache_dir();
    fs::create_dir_all(&base_dir)
        .map_err(|error| format!("Unable to create download cache directory: {error}"))?;
    let cache_file_name = cache_file_name_for_release_url(url);
    let target_path = base_dir.join(cache_file_name.clone());
    if let Ok(metadata) = fs::metadata(&target_path) {
        let existing_len = metadata.len();
        let has_expected_size = total_bytes
            .filter(|total| *total > 0)
            .map(|total| total == existing_len)
            .unwrap_or(existing_len > 0);
        if has_expected_size {
            emit_updater_event_payload(
                app,
                UpdaterProgressEvent {
                    run_id: run_id.map(str::to_string),
                    mode: mode.into(),
                    step: UpdaterStep::ResolveImage,
                    status: UpdaterStatus::Running,
                    message: format!("Using cached release image ({existing_len} bytes)."),
                    timestamp_epoch_ms: epoch_ms(),
                    stdout: None,
                    stderr: None,
                    exit_code: None,
                    duration_ms: None,
                    image_path: Some(path_to_string(&target_path)),
                    target_path: None,
                    progress_percent: Some(100.0),
                    bytes_written: Some(existing_len),
                    bytes_total: total_bytes.or(Some(existing_len)),
                },
            );
            return Ok(path_to_string(target_path));
        }
    }

    let _ = fs::remove_file(&target_path);
    let partial_path = base_dir.join(format!("{cache_file_name}.partial"));
    let _ = fs::remove_file(&partial_path);
    let mut file = fs::File::create(&partial_path)
        .map_err(|error| format!("Unable to create download file: {error}"))?;

    let mut bytes_written = 0u64;
    let mut last_emitted_bytes = 0u64;
    let mut last_emit = Instant::now();
    let mut buffer = [0u8; 256 * 1024];
    loop {
        if is_update_cancel_requested_for_session(run_id, mode) {
            let _ = fs::remove_file(&partial_path);
            return Err("Update canceled by user.".to_string());
        }

        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("Failed while reading release image download: {error}"))?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .map_err(|error| format!("Failed while writing downloaded release image: {error}"))?;
        bytes_written += read as u64;

        if bytes_written.saturating_sub(last_emitted_bytes) < 4 * 1024 * 1024
            && last_emit.elapsed() < Duration::from_millis(400)
        {
            continue;
        }

        last_emitted_bytes = bytes_written;
        last_emit = Instant::now();
        let progress_percent = total_bytes
            .filter(|total| *total > 0)
            .map(|total| ((bytes_written as f64 / total as f64) * 100.0).clamp(0.0, 100.0));
        emit_updater_event_payload(
            app,
            UpdaterProgressEvent {
                run_id: run_id.map(str::to_string),
                mode: mode.into(),
                step: UpdaterStep::ResolveImage,
                status: UpdaterStatus::Running,
                message: format!("Downloading release image ({bytes_written} bytes)."),
                timestamp_epoch_ms: epoch_ms(),
                stdout: None,
                stderr: None,
                exit_code: None,
                duration_ms: None,
                image_path: None,
                target_path: None,
                progress_percent,
                bytes_written: Some(bytes_written),
                bytes_total: total_bytes,
            },
        );
    }

    let final_progress_percent = total_bytes
        .filter(|total| *total > 0)
        .map(|total| ((bytes_written as f64 / total as f64) * 100.0).clamp(0.0, 100.0));
    emit_updater_event_payload(
        app,
        UpdaterProgressEvent {
            run_id: run_id.map(str::to_string),
            mode: mode.into(),
            step: UpdaterStep::ResolveImage,
            status: UpdaterStatus::Running,
            message: format!("Downloading release image ({bytes_written} bytes)."),
            timestamp_epoch_ms: epoch_ms(),
            stdout: None,
            stderr: None,
            exit_code: None,
            duration_ms: None,
            image_path: None,
            target_path: None,
            progress_percent: final_progress_percent,
            bytes_written: Some(bytes_written),
            bytes_total: total_bytes,
        },
    );

    file.flush()
        .map_err(|error| format!("Failed while finalizing downloaded release image: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Failed while syncing downloaded release image: {error}"))?;
    fs::rename(&partial_path, &target_path).map_err(|error| {
        format!("Failed while moving downloaded release image into cache: {error}")
    })?;

    Ok(path_to_string(target_path))
}
