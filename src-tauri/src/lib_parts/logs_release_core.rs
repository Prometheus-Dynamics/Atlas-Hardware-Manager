#[path = "logs_release_core/release_image_resolution.rs"]
mod release_image_resolution;
pub(crate) use release_image_resolution::{
    clear_release_download_cache, resolve_install_image_path,
};

fn extract_log_text_from_response(payload: &[u8], content_type: Option<&str>) -> Option<String> {
    if payload.is_empty() {
        return None;
    }

    if is_zip_payload(payload, content_type) {
        if let Some(text) = decode_zip_log_payload(payload) {
            return Some(text);
        }
    }

    if looks_like_json_payload(payload, content_type) {
        if let Ok(value) = serde_json::from_slice::<Value>(payload) {
            if let Some(text) = extract_log_text_from_json(&value) {
                return Some(text);
            }
        }
    }

    let text = String::from_utf8_lossy(payload).to_string();
    if text.trim().is_empty() || looks_like_html_document(&text, content_type) {
        None
    } else {
        Some(text)
    }
}

fn is_zip_payload(payload: &[u8], content_type: Option<&str>) -> bool {
    if payload.len() >= 4 && payload[0] == b'P' && payload[1] == b'K' {
        return true;
    }
    content_type
        .map(|value| value.to_ascii_lowercase().contains("zip"))
        .unwrap_or(false)
}

fn looks_like_json_payload(payload: &[u8], content_type: Option<&str>) -> bool {
    if content_type
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false)
    {
        return true;
    }

    let first_non_ws = payload
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace());
    matches!(first_non_ws, Some(b'{') | Some(b'['))
}

fn looks_like_html_document(text: &str, content_type: Option<&str>) -> bool {
    let lower_content_type = content_type
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let html_content_type =
        lower_content_type.contains("text/html")
            || lower_content_type.contains("application/xhtml+xml");

    // Only inspect a prefix of large payloads for markers.
    let prefix_len = text
        .char_indices()
        .nth(8192)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let lower = text[..prefix_len].to_ascii_lowercase();
    let has_document_markers = lower.contains("<!doctype html")
        || (lower.contains("<html") && lower.contains("<body"))
        || (lower.contains("<head") && lower.contains("</html>"));
    let has_single_page_app_markers = lower.contains("data-sveltekit-preload-data")
        || lower.contains("/_app/immutable/entry/start")
        || lower.contains("/_app/env.js");

    has_document_markers && (html_content_type || has_single_page_app_markers)
}

fn decode_zip_log_payload(payload: &[u8]) -> Option<String> {
    let cursor = io::Cursor::new(payload);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;
    let mut sections = Vec::new();

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).ok()?;
        if file.is_dir() {
            continue;
        }

        let name = file.name().to_string();
        let lower_name = name.to_ascii_lowercase();
        if !(lower_name.ends_with(".txt")
            || lower_name.ends_with(".log")
            || lower_name.contains("journal")
            || lower_name.contains("log"))
        {
            continue;
        }

        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes).to_string();
        if text.trim().is_empty() {
            continue;
        }

        sections.push(format!("===== {name} =====\n{text}"));
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn extract_log_text_from_json(payload: &Value) -> Option<String> {
    if payload.is_array() {
        return json_value_to_log_text(payload);
    }

    let keys = [
        "logs", "log", "entries", "lines", "messages", "journal", "stdout", "stderr",
    ];
    for key in keys {
        if let Some(value) = payload.get(key) {
            if let Some(text) = json_value_to_log_text(value) {
                return Some(text);
            }
        }
    }

    let mut fields = Vec::new();
    collect_scalar_fields(payload, String::new(), &mut fields);
    let mut lines = Vec::new();
    for (key, value) in fields {
        let key_lower = key.to_ascii_lowercase();
        if key_lower.contains("log")
            || key_lower.contains("journal")
            || key_lower.contains("message")
            || key_lower.contains("stderr")
            || key_lower.contains("stdout")
        {
            lines.push(format!("{key}: {value}"));
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn json_value_to_log_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(items) => {
            let lines = items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => {
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        }
                    }
                    Value::Object(_) | Value::Array(_) => serde_json::to_string(item).ok(),
                    _ => scalar_value_to_string(item),
                })
                .collect::<Vec<_>>();
            if lines.is_empty() {
                None
            } else {
                Some(lines.join("\n"))
            }
        }
        Value::Object(_) => serde_json::to_string_pretty(value).ok(),
        _ => scalar_value_to_string(value),
    }
}

fn constrain_log_lines(text: &str, max_lines: usize) -> (String, usize, bool) {
    let mut lines = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let line_count = lines.len();
    let truncated = line_count > max_lines;
    if truncated {
        lines = lines
            .into_iter()
            .rev()
            .take(max_lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    let mut normalized = lines.join("\n");
    const MAX_TEXT_BYTES: usize = 180_000;
    if normalized.len() > MAX_TEXT_BYTES {
        let keep_from = normalized.len().saturating_sub(MAX_TEXT_BYTES);
        normalized = normalized.split_off(keep_from);
    }

    (normalized, line_count, truncated)
}

fn fetch_release_images_from_github() -> Result<Vec<ReleaseImageOption>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|error| format!("Unable to build HTTP client: {error}"))?;

    let response = client
        .get(HELIOS_RELEASES_API_URL)
        .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|error| format!("Failed to fetch GitHub releases: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "GitHub releases API returned status {}",
            response.status()
        ));
    }

    let payload: Value = response
        .json()
        .map_err(|error| format!("Failed to parse GitHub releases payload: {error}"))?;

    let mut options = Vec::new();
    let Some(releases) = payload.as_array() else {
        return Ok(options);
    };

    for release in releases {
        let release_tag = release
            .get("tag_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let release_name = release
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&release_tag)
            .to_string();
        let prerelease = release
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let published_at = release
            .get("published_at")
            .and_then(Value::as_str)
            .map(str::to_string);

        let Some(assets) = release.get("assets").and_then(Value::as_array) else {
            continue;
        };

        for asset in assets {
            let asset_name = asset
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !is_supported_image_name(&asset_name) {
                continue;
            }

            let download_url = asset
                .get("browser_download_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if download_url.is_empty() {
                continue;
            }

            options.push(ReleaseImageOption {
                release_tag: release_tag.clone(),
                release_name: release_name.clone(),
                prerelease,
                asset_name,
                download_url,
                size_bytes: asset
                    .get("size")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                published_at: published_at.clone(),
            });
        }
    }

    Ok(options)
}

fn select_flash_target_after_rpiboot(before_paths: &HashSet<String>) -> Result<String, String> {
    let start = Instant::now();
    let timeout = Duration::from_secs(30);

    loop {
        if is_update_cancel_requested() {
            return Err("Update canceled by user.".to_string());
        }
        let targets = discover_flash_targets()?;
        let mut new_targets = targets
            .iter()
            .filter(|target| !before_paths.contains(&target.path))
            .map(|target| target.path.clone())
            .collect::<Vec<_>>();

        if new_targets.len() == 1 {
            return Ok(new_targets.remove(0));
        }

        if new_targets.len() > 1 {
            return Err(
                "Multiple new flash targets were detected after rpiboot. Disconnect extra removable drives and retry."
                    .to_string(),
            );
        }

        if start.elapsed() >= timeout {
            return Err(
                "No unique flash target appeared after rpiboot. Check USB connection and retry."
                    .to_string(),
            );
        }

        thread::sleep(Duration::from_millis(800));
    }
}

#[cfg(target_os = "linux")]
fn collect_lsblk_labels(entry: &Value, labels: &mut Vec<String>) {
    if let Some(label) = entry.get("label").and_then(Value::as_str) {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            labels.push(trimmed.to_string());
        }
    }

    if let Some(children) = entry.get("children").and_then(Value::as_array) {
        for child in children {
            collect_lsblk_labels(child, labels);
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_target_has_helios_partition_signature(path: &str) -> bool {
    let Some(lsblk_path) = find_in_path("lsblk") else {
        return false;
    };

    let output = match Command::new(lsblk_path)
        .args(["-J", "-o", "PATH,LABEL"])
        .arg(path)
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }

    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let mut labels = Vec::new();
    if let Some(block_devices) = value.get("blockdevices").and_then(Value::as_array) {
        for device in block_devices {
            collect_lsblk_labels(device, &mut labels);
        }
    }

    let labels_upper = labels
        .into_iter()
        .map(|label| label.to_ascii_uppercase())
        .collect::<HashSet<_>>();
    labels_upper.contains("BOOT")
        && (labels_upper.contains("ACTIVE")
            || labels_upper.contains("RESERVE")
            || labels_upper.contains("DATA"))
}

fn looks_like_helios_flash_target(target: &FlashTarget) -> bool {
    let model = target.model.trim().to_ascii_lowercase();
    if model.contains("raspberry pi multi-function usb device")
        || model.contains("raspberry pi")
        || model.contains("helios")
    {
        return true;
    }

    #[cfg(target_os = "linux")]
    {
        return linux_target_has_helios_partition_signature(&target.path);
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn select_existing_flash_target(targets: &[FlashTarget]) -> Result<String, String> {
    if targets.is_empty() {
        return Err(
            "No flash target was found and no bootloader device is present. Put the device into USB bootloader mode and retry."
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = targets
            .iter()
            .filter(|target| looks_like_helios_flash_target(target))
            .collect::<Vec<_>>();
        return match candidates.as_slice() {
            [target] => Ok(target.path.clone()),
            [] => Err(
                "No HeliOS/Raspberry Pi flash target signature was found. Put the device into USB bootloader mode (or disconnect unrelated USB storage) and retry."
                    .to_string(),
            ),
            _ => Err(
                "Multiple HeliOS-like flash targets were detected. Disconnect extra removable drives and retry so only one target remains."
                    .to_string(),
            ),
        };
    }

    #[cfg(not(target_os = "linux"))]
    {
        match targets {
            [target] => Ok(target.path.clone()),
            _ => Err(
                "Multiple removable flash targets are connected. Disconnect extra USB storage and retry so only the HeliOS target remains."
                    .to_string(),
            ),
        }
    }
}

fn skipped_rpiboot_result(message: &str) -> OperationResult {
    OperationResult {
        success: true,
        exit_code: None,
        duration_ms: 0,
        stdout: String::new(),
        stderr: String::new(),
        message: message.to_string(),
        timed_out: false,
    }
}
