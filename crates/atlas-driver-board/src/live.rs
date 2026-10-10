//! Optional live endpoints a device may list in its identity document:
//!
//! ```json
//! "endpoints": { "metrics": "/api/metrics", "logs": "/api/logs", "actions": "/api/actions" },
//! "actions": [ { "id": "locate", "label": "Find it" } ],
//! "camera_stream": "/stream.mjpg"
//! ```
//!
//! None of these are required. Atlas offers a feature only when the device
//! lists its endpoint, and reads the answers leniently so a device can start
//! simple: metrics may be a list of metric objects or a plain `{"cpu": 12}`
//! map, logs a list of line objects or of strings.

use async_trait::async_trait;
use atlas_driver::{
    ActionsCapability, DeviceAction, DriverError, Identity, LogLevel, LogLine, LogsCapability,
    Metric, TelemetryCapability,
};
use serde_json::Value;

/// Resolves `path` against the identity URL: absolute URLs stay as they are,
/// paths are joined to the identity URL's origin.
pub(crate) fn resolve(identity_url: &str, path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return Some(path.to_string());
    }
    let scheme_end = identity_url.find("://")? + 3;
    let origin_end = identity_url[scheme_end..]
        .find('/')
        .map_or(identity_url.len(), |index| scheme_end + index);
    let origin = &identity_url[..origin_end];
    // `:<port>/path`: the same host, another port (the board's stream).
    if let Some(rest) = path.strip_prefix(':') {
        let (port, tail) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let authority = &origin[scheme_end..];
        // The host without its port; an IPv6 host keeps its brackets.
        let host = match authority.rfind(':') {
            Some(colon) if !authority[colon..].contains(']') => &authority[..colon],
            _ => authority,
        };
        return Some(format!("{}{host}:{port}{tail}", &origin[..scheme_end]));
    }
    Some(if path.starts_with('/') {
        format!("{origin}{path}")
    } else {
        format!("{origin}/{path}")
    })
}

fn label_for(id: &str) -> String {
    let mut label = id.replace(['_', '-', '.'], " ");
    if let Some(first) = label.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    label
}

/// Accepts a list of metric objects, `{"metrics": [...]}`, or a flat map of
/// numbers. Anything unreadable is skipped rather than failing the read.
pub(crate) fn parse_metrics(value: Value) -> Vec<Metric> {
    let value = match value {
        Value::Object(mut map) if map.contains_key("metrics") => {
            map.remove("metrics").unwrap_or(Value::Null)
        }
        other => other,
    };
    match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| {
                let mut metric = item.clone();
                if let Value::Object(map) = &mut metric {
                    let id = map.get("id")?.as_str()?.to_string();
                    map.entry("label").or_insert_with(|| label_for(&id).into());
                }
                serde_json::from_value::<Metric>(metric).ok()
            })
            .collect(),
        Value::Object(map) => map
            .into_iter()
            .filter_map(|(id, value)| {
                let number = value.as_f64()?;
                Some(Metric::new(&id, &label_for(&id), number, None))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_level(text: &str) -> LogLevel {
    match text.to_ascii_lowercase().as_str() {
        "debug" | "trace" => LogLevel::Debug,
        "warn" | "warning" => LogLevel::Warning,
        "error" | "err" | "fatal" | "critical" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// Accepts a list of line objects or strings, or `{"lines": [...]}`.
pub(crate) fn parse_logs(value: Value) -> Vec<LogLine> {
    let value = match value {
        Value::Object(mut map) if map.contains_key("lines") => {
            map.remove("lines").unwrap_or(Value::Null)
        }
        other => other,
    };
    let Value::Array(items) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| match item {
            Value::String(message) => Some(LogLine {
                at_ms: None,
                level: LogLevel::Info,
                source: None,
                message,
            }),
            Value::Object(map) => {
                let message = map
                    .get("message")
                    .or_else(|| map.get("msg"))?
                    .as_str()?
                    .to_string();
                Some(LogLine {
                    at_ms: map
                        .get("at_ms")
                        .or_else(|| map.get("ts"))
                        .and_then(Value::as_u64),
                    level: map
                        .get("level")
                        .and_then(Value::as_str)
                        .map_or(LogLevel::Info, parse_level),
                    source: map
                        .get("source")
                        .or_else(|| map.get("unit"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    message,
                })
            }
            _ => None,
        })
        .collect()
}

pub(crate) async fn get_json(http: &reqwest::Client, url: &str) -> Result<Value, DriverError> {
    http.get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| DriverError::Unreachable(error.to_string()))?
        .json::<Value>()
        .await
        .map_err(|error| DriverError::Other(format!("{url} sent something unreadable: {error}")))
}

pub(crate) struct BoardTelemetry {
    pub(crate) http: reqwest::Client,
    pub(crate) url: String,
}

#[async_trait]
impl TelemetryCapability for BoardTelemetry {
    async fn read(&self, _device: &Identity) -> Result<Vec<Metric>, DriverError> {
        Ok(parse_metrics(get_json(&self.http, &self.url).await?))
    }
}

pub(crate) struct BoardLogs {
    pub(crate) http: reqwest::Client,
    pub(crate) url: String,
}

#[async_trait]
impl LogsCapability for BoardLogs {
    async fn tail(&self, _device: &Identity, lines: usize) -> Result<Vec<LogLine>, DriverError> {
        let separator = if self.url.contains('?') { '&' } else { '?' };
        let url = format!("{}{separator}lines={lines}", self.url);
        let mut parsed = parse_logs(get_json(&self.http, &url).await?);
        let excess = parsed.len().saturating_sub(lines);
        parsed.drain(..excess);
        Ok(parsed)
    }
}

pub(crate) struct BoardActions {
    pub(crate) http: reqwest::Client,
    pub(crate) url: String,
    pub(crate) actions: Vec<DeviceAction>,
}

#[async_trait]
impl ActionsCapability for BoardActions {
    fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
        self.actions.clone()
    }

    /// `POST <actions>/<id>`; any 2xx answer counts as done.
    async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
        let url = format!("{}/{action_id}", self.url.trim_end_matches('/'));
        self.http
            .post(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn endpoints_resolve_against_the_identity_origin() {
        let base = "http://10.0.0.5:5800/.well-known/pd-device";
        assert_eq!(
            resolve(base, "/api/metrics").as_deref(),
            Some("http://10.0.0.5:5800/api/metrics")
        );
        assert_eq!(
            resolve(base, "api/logs").as_deref(),
            Some("http://10.0.0.5:5800/api/logs")
        );
        assert_eq!(
            resolve(base, "https://cam.local/x").as_deref(),
            Some("https://cam.local/x")
        );
        assert_eq!(
            resolve(base, ":5898/stream").as_deref(),
            Some("http://10.0.0.5:5898/stream")
        );
        assert_eq!(
            resolve(
                "http://[fe80::1%25usb0]:5899/.well-known/pd-device",
                ":5898/stream"
            )
            .as_deref(),
            Some("http://[fe80::1%25usb0]:5898/stream")
        );
        assert_eq!(
            resolve("http://board.local/.well-known/pd-device", ":5898/stream").as_deref(),
            Some("http://board.local:5898/stream")
        );
        assert_eq!(resolve(base, ":x/stream"), None);
        assert_eq!(resolve(base, " "), None);
        assert_eq!(resolve("not a url", "/x"), None);
    }

    #[test]
    fn metrics_can_be_a_plain_map() {
        let metrics = parse_metrics(json!({ "cpu": 12.5, "temp": 54, "note": "hi" }));
        assert_eq!(metrics.len(), 2);
        assert!(metrics.iter().any(|m| m.id == "cpu" && m.label == "Cpu"));
    }

    #[test]
    fn metrics_can_be_full_objects_and_bad_ones_are_skipped() {
        let metrics = parse_metrics(json!({ "metrics": [
            { "id": "temp", "label": "SoC", "value": 61.0, "unit": "°C", "warn_above": 75 },
            { "id": "fan", "value": 3200 },
            { "label": "no id", "value": 1 },
            { "id": "broken", "value": "x" }
        ]}));
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].unit.as_deref(), Some("°C"));
        assert_eq!(metrics[1].label, "Fan");
    }

    #[test]
    fn logs_accept_strings_and_objects() {
        let lines = parse_logs(json!({ "lines": [
            "plain line",
            { "ts": 5, "level": "WARN", "unit": "photonvision", "msg": "hot" },
            42
        ]}));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].level, LogLevel::Warning);
        assert_eq!(lines[1].source.as_deref(), Some("photonvision"));
        assert_eq!(lines[1].at_ms, Some(5));
    }
}
