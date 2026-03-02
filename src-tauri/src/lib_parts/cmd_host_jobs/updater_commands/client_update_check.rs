use super::*;

#[derive(Debug, Clone)]
struct SelectedReleaseAsset {
    name: String,
    url: String,
}

fn normalize_version_for_compare(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('-')
        .next()
        .unwrap_or("")
        .trim();
    if core.is_empty() {
        return None;
    }

    let mut components = Vec::new();
    for segment in core.split('.') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            return None;
        }
        let value = trimmed.parse::<u64>().ok()?;
        components.push(value);
    }
    if components.is_empty() {
        return None;
    }
    Some(components)
}

fn is_version_newer(candidate: &str, current: &str) -> Option<bool> {
    let candidate_components = normalize_version_for_compare(candidate)?;
    let current_components = normalize_version_for_compare(current)?;
    let max_len = candidate_components.len().max(current_components.len());
    for index in 0..max_len {
        let candidate_value = *candidate_components.get(index).unwrap_or(&0);
        let current_value = *current_components.get(index).unwrap_or(&0);
        if candidate_value != current_value {
            return Some(candidate_value > current_value);
        }
    }
    Some(false)
}

fn normalized_release_version_label(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('-')
        .next()
        .unwrap_or("")
        .trim();
    if trimmed.is_empty() || normalize_version_for_compare(trimmed).is_none() {
        return None;
    }
    Some(trimmed.to_string())
}

fn release_asset_name_is_downloadable(asset_name: &str) -> bool {
    let lower = asset_name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    let excluded_suffixes = [
        ".sig", ".sha256", ".sha512", ".txt", ".json", ".yaml", ".yml",
    ];
    if excluded_suffixes
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        return false;
    }
    if lower.contains("source code") || lower.contains("checksums") {
        return false;
    }
    true
}

fn release_assets(release: &Value) -> Vec<SelectedReleaseAsset> {
    let mut assets = Vec::new();
    let Some(items) = release.get("assets").and_then(Value::as_array) else {
        return assets;
    };

    for asset in items {
        let Some(name) = asset
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(url) = asset
            .get("browser_download_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !release_asset_name_is_downloadable(name) {
            continue;
        }
        assets.push(SelectedReleaseAsset {
            name: name.to_string(),
            url: url.to_string(),
        });
    }

    assets
}

fn asset_matches_current_arch(name_lower: &str, arch: &str) -> bool {
    let known_markers = ["x86_64", "x64", "amd64", "aarch64", "arm64"];
    let contains_known_marker = known_markers
        .iter()
        .any(|marker| name_lower.contains(marker));

    let arch_markers: &[&str] = match arch {
        "x86_64" => &["x86_64", "x64", "amd64"],
        "aarch64" => &["aarch64", "arm64"],
        _ => &[],
    };

    if arch_markers.is_empty() {
        return true;
    }
    if arch_markers
        .iter()
        .any(|marker| name_lower.contains(marker))
    {
        return true;
    }
    !contains_known_marker
}

fn platform_asset_preference_score(name_lower: &str) -> Option<u8> {
    match env::consts::OS {
        "windows" => {
            if name_lower.ends_with("-setup.exe") || name_lower.contains("_setup.exe") {
                return Some(0);
            }
            if name_lower.ends_with(".msi") {
                return Some(1);
            }
            if name_lower.ends_with(".exe") {
                return Some(2);
            }
            None
        }
        "macos" => {
            if name_lower.ends_with(".dmg") {
                return Some(0);
            }
            if name_lower.ends_with(".pkg") {
                return Some(1);
            }
            if name_lower.ends_with(".app.tar.gz") {
                return Some(2);
            }
            if name_lower.ends_with(".zip") {
                return Some(3);
            }
            None
        }
        "linux" => {
            if name_lower.ends_with(".deb") || name_lower.ends_with(".rpm") {
                return None;
            }
            if name_lower.contains("unknown-linux-gnu-release") {
                return Some(0);
            }
            if name_lower.ends_with(".appimage") {
                return Some(1);
            }
            if name_lower.ends_with(".tar.gz") {
                return Some(2);
            }
            if name_lower.ends_with(".run") || name_lower.ends_with(".sh") {
                return Some(3);
            }
            if name_lower == "atlas-hardware-manager" {
                return Some(4);
            }
            None
        }
        _ => None,
    }
}

fn select_platform_release_asset(release: &Value) -> Option<SelectedReleaseAsset> {
    let arch = env::consts::ARCH;
    let candidates = release_assets(release);
    let mut best: Option<(u8, SelectedReleaseAsset)> = None;

    for asset in candidates {
        let lower = asset.name.to_ascii_lowercase();
        if !asset_matches_current_arch(&lower, arch) {
            continue;
        }
        let Some(score) = platform_asset_preference_score(&lower) else {
            continue;
        };
        match best.as_ref() {
            Some((best_score, _)) if *best_score <= score => {}
            _ => best = Some((score, asset)),
        }
    }

    best.map(|(_, asset)| asset)
}

fn sanitize_asset_file_name(file_name: &str) -> String {
    let sanitized = file_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('-').trim();
    if trimmed.is_empty() {
        "atlas-client-update.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_asset_name_from_download_url(url: &str) -> Option<String> {
    let raw = url.split('?').next().unwrap_or(url);
    raw.rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(sanitize_asset_file_name)
}

fn client_update_download_dir() -> PathBuf {
    env::temp_dir()
        .join("atlas-hardware-manager")
        .join("client-updates")
}

fn download_client_update_asset(url: &str, asset_name: &str) -> Result<PathBuf, String> {
    let download_dir = client_update_download_dir();
    fs::create_dir_all(&download_dir)
        .map_err(|error| format!("Unable to create update download directory: {error}"))?;

    let file_name = sanitize_asset_file_name(asset_name);
    let temp_path = download_dir.join(format!("{file_name}.part"));
    let final_path = download_dir.join(file_name);

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| format!("Unable to build update download client: {error}"))?;
    let mut response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
        .send()
        .map_err(|error| format!("Unable to download update asset: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Update asset download returned status {}.",
            response.status()
        ));
    }

    let mut temp_file = fs::File::create(&temp_path)
        .map_err(|error| format!("Unable to create temporary update file: {error}"))?;
    io::copy(&mut response, &mut temp_file)
        .map_err(|error| format!("Unable to write downloaded update asset: {error}"))?;
    temp_file
        .sync_all()
        .map_err(|error| format!("Unable to finalize update download: {error}"))?;
    fs::rename(&temp_path, &final_path)
        .map_err(|error| format!("Unable to place downloaded update asset: {error}"))?;
    Ok(final_path)
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .map_err(|error| format!("Unable to inspect downloaded updater permissions: {error}"))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("Unable to mark downloaded updater as executable: {error}"))?;
    Ok(())
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn launch_client_update_installer(installer_path: &Path, asset_name: &str) -> Result<String, String> {
    let lower = asset_name.to_ascii_lowercase();

    #[cfg(target_os = "windows")]
    {
        if lower.ends_with(".msi") {
            Command::new("msiexec")
                .args(["/i", &path_to_string(installer_path)])
                .spawn()
                .map_err(|error| format!("Unable to launch MSI installer: {error}"))?;
            return Ok("Launched Windows MSI installer. Follow the installer prompts.".to_string());
        }
        Command::new(installer_path)
            .spawn()
            .map_err(|error| format!("Unable to launch Windows updater executable: {error}"))?;
        return Ok("Launched Windows updater executable. Follow the installer prompts.".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(installer_path)
            .spawn()
            .map_err(|error| format!("Unable to open downloaded macOS installer: {error}"))?;
        return Ok("Opened downloaded macOS installer. Complete installation from Installer/Finder.".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        if lower.ends_with(".deb") || lower.ends_with(".rpm") {
            return Err("Automatic install for package-managed Linux bundles is not supported in-app for this distro. Open the release page and install manually.".to_string());
        }
        ensure_executable(installer_path)?;
        Command::new(installer_path)
            .spawn()
            .map_err(|error| format!("Unable to launch downloaded Linux updater: {error}"))?;
        return Ok("Launched downloaded Linux updater binary. If prompted, close Atlas and continue the installer.".to_string());
    }

    #[allow(unreachable_code)]
    Err("In-app self-update launch is not supported on this platform.".to_string())
}

fn fetch_client_update_status() -> Result<ClientUpdateStatus, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let checked_at_epoch_ms = epoch_ms();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("Unable to build HTTP client: {error}"))?;
    let response = client
        .get(ATLAS_RELEASES_API_URL)
        .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|error| format!("Failed to fetch Atlas releases: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub releases API returned status {}",
            response.status()
        ));
    }
    let payload: Value = response
        .json()
        .map_err(|error| format!("Failed to parse Atlas releases payload: {error}"))?;
    let Some(releases) = payload.as_array() else {
        return Err("GitHub releases API returned an unexpected payload.".to_string());
    };
    let latest_release = releases
        .iter()
        .find(|release| {
            !release
                .get("draft")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && !release
                    .get("prerelease")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .or_else(|| {
            releases.iter().find(|release| {
                !release
                    .get("draft")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
        });

    let Some(latest_release) = latest_release else {
        return Ok(ClientUpdateStatus {
            current_version,
            latest_version: None,
            latest_tag: None,
            latest_name: None,
            prerelease: false,
            update_available: false,
            download_asset_name: None,
            download_url: None,
            release_page_url: None,
            published_at: None,
            checked_at_epoch_ms,
        });
    };

    let selected_asset = select_platform_release_asset(latest_release);
    let latest_tag = latest_release
        .get("tag_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let latest_name = latest_release
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let latest_version = latest_tag
        .as_deref()
        .and_then(normalized_release_version_label)
        .or_else(|| latest_name.as_deref().and_then(normalized_release_version_label));
    let update_available = latest_version
        .as_deref()
        .and_then(|latest| is_version_newer(latest, &current_version))
        .unwrap_or(false);

    Ok(ClientUpdateStatus {
        current_version,
        latest_version,
        latest_tag,
        latest_name,
        prerelease: latest_release
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        update_available,
        download_asset_name: selected_asset.as_ref().map(|asset| asset.name.clone()),
        download_url: selected_asset.as_ref().map(|asset| asset.url.clone()),
        release_page_url: latest_release
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        published_at: latest_release
            .get("published_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        checked_at_epoch_ms,
    })
}

pub(crate) fn get_client_update_status_internal() -> Result<ClientUpdateStatus, String> {
    fetch_client_update_status()
}

pub(crate) fn start_client_self_update_internal() -> Result<ClientSelfUpdateResult, String> {
    let status = fetch_client_update_status()?;
    if !status.update_available {
        return Ok(ClientSelfUpdateResult {
            success: true,
            started: false,
            message: "Atlas is already up to date.".to_string(),
            current_version: status.current_version,
            latest_version: status.latest_version,
            download_asset_name: status.download_asset_name,
            download_url: status.download_url,
            release_page_url: status.release_page_url,
            installer_path: None,
        });
    }

    let Some(download_url) = status.download_url.clone() else {
        return Ok(ClientSelfUpdateResult {
            success: false,
            started: false,
            message:
                "No compatible update installer was detected for this platform build. Opening the release page is required."
                    .to_string(),
            current_version: status.current_version,
            latest_version: status.latest_version,
            download_asset_name: status.download_asset_name,
            download_url: None,
            release_page_url: status.release_page_url,
            installer_path: None,
        });
    };
    let download_asset_name = status
        .download_asset_name
        .clone()
        .or_else(|| parse_asset_name_from_download_url(&download_url))
        .ok_or_else(|| "Unable to determine update asset name from release metadata.".to_string())?;

    let installer_path = download_client_update_asset(&download_url, &download_asset_name)?;
    let launch_message = match launch_client_update_installer(&installer_path, &download_asset_name) {
        Ok(message) => message,
        Err(error) => {
            return Ok(ClientSelfUpdateResult {
                success: false,
                started: false,
                message: format!(
                    "{error} Open the release page and run the installer manually."
                ),
                current_version: status.current_version,
                latest_version: status.latest_version,
                download_asset_name: Some(download_asset_name),
                download_url: Some(download_url),
                release_page_url: status.release_page_url,
                installer_path: Some(path_to_string(&installer_path)),
            });
        }
    };

    Ok(ClientSelfUpdateResult {
        success: true,
        started: true,
        message: launch_message,
        current_version: status.current_version,
        latest_version: status.latest_version,
        download_asset_name: Some(download_asset_name),
        download_url: Some(download_url),
        release_page_url: status.release_page_url,
        installer_path: Some(path_to_string(&installer_path)),
    })
}
