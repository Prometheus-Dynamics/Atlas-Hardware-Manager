use super::*;

pub(crate) fn ota_api_base_urls(target_ip: &str) -> Vec<String> {
    let mut candidates = vec![
        format!("http://{target_ip}:5801"),
        format!("http://{target_ip}:5800"),
        format!("http://{target_ip}:80"),
        format!("http://{target_ip}"),
    ];
    candidates.retain(|value| !value.trim().is_empty());
    let mut deduped = Vec::new();
    for candidate in candidates {
        if !deduped.contains(&candidate) {
            deduped.push(candidate);
        }
    }
    deduped
}

pub(crate) fn fetch_ota_runtime_state(
    client: &reqwest::blocking::Client,
    api_bases: &[String],
) -> Result<OtaRuntimeState, String> {
    let mut errors = Vec::new();
    for base in api_bases {
        let url = format!("{base}/v1/ota/state");
        let response = match client
            .get(&url)
            .header(reqwest::header::USER_AGENT, "Atlas-Hardware-Manager")
            .header(reqwest::header::ACCEPT, "application/json,text/plain,*/*")
            .send()
        {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!("{url}: {error}"));
                continue;
            }
        };
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "<no response body>".to_string());
            errors.push(format!("{url}: HTTP {status} ({body})"));
            continue;
        }
        let raw_state = match response.json::<Value>() {
            Ok(value) => value,
            Err(error) => {
                errors.push(format!("{url}: invalid JSON response ({error})"));
                continue;
            }
        };
        return Ok(parse_ota_runtime_state(Some(&raw_state)));
    }

    if errors.is_empty() {
        Err("No OTA state endpoints were available.".to_string())
    } else {
        Err(errors.join(" | "))
    }
}

pub(crate) fn parse_ota_runtime_state(raw: Option<&Value>) -> OtaRuntimeState {
    let Some(raw) = raw else {
        return OtaRuntimeState::default();
    };
    let root = match raw {
        Value::Object(root) => root,
        _ => return OtaRuntimeState::default(),
    };

    let read_text_from =
        |source: &serde_json::Map<String, Value>, keys: &[&str]| -> Option<String> {
            for key in keys {
                if let Some(value) = source.get(*key) {
                    let parsed = match value {
                        Value::String(text) => Some(text.trim().to_string()),
                        Value::Number(number) => Some(number.to_string()),
                        Value::Bool(flag) => Some(flag.to_string()),
                        _ => None,
                    }
                    .filter(|text| !text.is_empty());
                    if parsed.is_some() {
                        return parsed;
                    }
                }
            }
            None
        };
    let read_progress_from =
        |source: &serde_json::Map<String, Value>, keys: &[&str]| -> Option<f64> {
            for key in keys {
                if let Some(value) = source.get(*key) {
                    if let Some(parsed) = parse_ota_runtime_progress_value(value, key) {
                        return Some(parsed);
                    }
                }
            }
            None
        };

    let sources = collect_ota_runtime_state_sources(root);

    let read_text = |keys: &[&str]| -> Option<String> {
        for source in &sources {
            if let Some(value) = read_text_from(source, keys) {
                return Some(value);
            }
        }
        None
    };
    let read_progress = |keys: &[&str]| -> Option<f64> {
        for source in &sources {
            if let Some(value) = read_progress_from(source, keys) {
                return Some(value);
            }
        }
        None
    };

    OtaRuntimeState {
        update_id: read_text(&["update_id", "updateId", "id"]),
        stage: read_text(&[
            "stage",
            "status",
            "phase",
            "state",
            "state_name",
            "stateName",
            "update_stage",
            "updateStage",
            "apply_state",
            "applyState",
            "ota_stage",
            "otaStage",
            "current_stage",
            "currentStage",
        ])
        .map(normalize_ota_stage_name),
        progress_percent: read_progress(&[
            "progress_percent",
            "progressPercent",
            "progress",
            "percent",
            "pct",
            "stage_percent",
            "stagePercent",
            "apply_progress",
            "applyProgress",
            "apply_percent",
            "applyPercent",
            "progress_ratio",
            "progressRatio",
            "progress_fraction",
            "progressFraction",
            "fraction_complete",
            "fractionComplete",
        ])
        .map(|value| value.clamp(0.0, 100.0)),
        last_error: read_text(&[
            "last_error",
            "lastError",
            "error",
            "error_message",
            "errorMessage",
            "reason",
            "detail",
            "message",
            "failure_reason",
            "failureReason",
        ]),
    }
}

pub(crate) fn collect_ota_runtime_state_sources(
    root: &serde_json::Map<String, Value>,
) -> Vec<&serde_json::Map<String, Value>> {
    const CHILD_KEYS: [&str; 15] = [
        "state",
        "active_update",
        "activeUpdate",
        "active",
        "ota",
        "update",
        "updater",
        "snapshot",
        "payload",
        "result",
        "data",
        "runtime",
        "status",
        "progress",
        "apply_progress",
    ];

    let mut sources = Vec::<&serde_json::Map<String, Value>>::new();
    let mut queue = vec![root];
    let mut seen = HashSet::<usize>::new();
    while let Some(source) = queue.pop() {
        let ptr = source as *const _ as usize;
        if !seen.insert(ptr) {
            continue;
        }
        sources.push(source);
        for key in CHILD_KEYS {
            if let Some(Value::Object(child)) = source.get(key) {
                queue.push(child);
            }
        }
    }
    sources
}
