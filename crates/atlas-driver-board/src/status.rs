//! A board's state and event log, from the read-only endpoints its identity
//! lists (`endpoints.status`, `endpoints.events`; device package
//! `/usr/lib/board/status --json` and the event log):
//!
//! ```json
//! GET /status  {"version":1,"time":..,"boot":{..},"failed_units":[..],
//!               "temperatures":[..],"fan":{..},"update":{..},
//!               "clock":{"time":..,"ntp_synchronized":..},"drift":{..},
//!               "hardware":{"at":..,"devices":[..]}}
//! GET /events?since=<t>&limit=<n>
//!              {"time":..,"boot_id":..,"events":[{"t":..,"boot_id":..,
//!               "kind":..,"source":..,"message":..,"data":{..}}]}
//! ```
//!
//! Both are read leniently: a part that doesn't parse is left out rather than
//! failing the read. Temperatures and the fan also become telemetry when the
//! board has no metrics endpoint.

use async_trait::async_trait;
use atlas_driver::{
    BootInfo, DeviceEvent, DeviceStatus, Drift, DriverError, EventPage, EventQuery, EventSource,
    FanState, HardwareSnapshot, Identity, Metric, StatusCapability, TelemetryCapability,
    Temperature, UpdateState, metric_ids,
};
use serde::Deserialize;
use serde_json::Value;

use crate::live::get_json;

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// `update.json` as the board writes it: empty strings mean "none".
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawUpdate {
    state: String,
    slot_active: Option<String>,
    slot_staged: Option<String>,
    version_active: Option<String>,
    version_staged: Option<String>,
    version_previous: Option<String>,
    progress: u32,
    error: Option<String>,
    started_by: Option<String>,
}

fn some(text: Option<String>) -> Option<String> {
    text.filter(|text| !text.trim().is_empty())
}

impl From<RawUpdate> for UpdateState {
    fn from(raw: RawUpdate) -> Self {
        Self {
            state: raw.state,
            slot_active: some(raw.slot_active),
            slot_staged: some(raw.slot_staged),
            version_active: some(raw.version_active),
            version_staged: some(raw.version_staged),
            version_previous: some(raw.version_previous),
            progress: raw.progress.min(1000),
            error: some(raw.error),
            started_by: some(raw.started_by).map(|by| EventSource::parse(&by)),
        }
    }
}

/// One part of the status, or `None` when it is missing or unreadable.
fn part<T: serde::de::DeserializeOwned>(value: &Value, key: &str) -> Option<T> {
    value
        .get(key)
        .filter(|part| !part.is_null())
        .and_then(|part| serde_json::from_value(part.clone()).ok())
}

/// The status document, with the clock offset taken against `now` (Unix
/// seconds on this computer).
pub(crate) fn parse_status(value: &Value, now: i64) -> DeviceStatus {
    let clock_time = value
        .get("clock")
        .and_then(|clock| clock.get("time"))
        .or_else(|| value.get("time"))
        .and_then(Value::as_i64);
    DeviceStatus {
        time: value.get("time").and_then(Value::as_i64),
        boot: part::<BootInfo>(value, "boot").map(|mut boot| {
            boot.id = some(boot.id);
            boot.slot = some(boot.slot).filter(|slot| slot != "unknown");
            boot.kernel = some(boot.kernel);
            boot
        }),
        // null (the board couldn't ask systemd) stays unknown.
        failed_units: part::<Vec<String>>(value, "failed_units"),
        temperatures: value
            .get("temperatures")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| serde_json::from_value::<Temperature>(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default(),
        fan: part::<FanState>(value, "fan"),
        update: part::<RawUpdate>(value, "update")
            .map(UpdateState::from)
            .filter(|update| !update.state.is_empty()),
        clock_offset_s: clock_time.map(|time| time - now),
        ntp_synchronized: value
            .get("clock")
            .and_then(|clock| clock.get("ntp_synchronized"))
            .and_then(Value::as_bool),
        drift: part::<Drift>(value, "drift"),
        hardware: part::<HardwareSnapshot>(value, "hardware"),
    }
}

/// `{"events":[..]}` or a bare list; events that don't parse are skipped.
/// The `/events` answer: its events, and the board's clock, boot and newest
/// seq when it says them.
pub(crate) fn parse_event_page(value: Value) -> EventPage {
    let time = value.get("time").and_then(Value::as_i64);
    let boot_id = value
        .get("boot_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let newest_seq = value.get("seq").and_then(Value::as_u64);
    EventPage {
        events: parse_events(value),
        time,
        boot_id,
        newest_seq,
    }
}

pub(crate) fn parse_events(value: Value) -> Vec<DeviceEvent> {
    let items = match value {
        Value::Object(mut map) => map.remove("events").unwrap_or(Value::Null),
        other => other,
    };
    let Value::Array(items) = items else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<DeviceEvent>(item).ok())
        .filter(|event| !event.kind.is_empty())
        .collect()
}

/// Temperatures, the fan and uptime as readings, for a board without a
/// metrics endpoint.
pub(crate) fn status_metrics(status: &DeviceStatus) -> Vec<Metric> {
    let mut metrics = Vec::new();
    let cpu = status
        .temperatures
        .iter()
        .find(|t| t.id.contains("cpu"))
        .or_else(|| status.temperatures.first());
    if let Some(cpu) = cpu {
        metrics.push(
            Metric::new(
                metric_ids::TEMPERATURE,
                "Temperature",
                cpu.celsius,
                Some("°C"),
            )
            .range(85.0, Some(75.0)),
        );
    }
    if let Some(fan) = &status.fan {
        if let Some(rpm) = fan.rpm {
            metrics.push(Metric::new(
                metric_ids::FAN,
                "Fan",
                f64::from(rpm),
                Some("rpm"),
            ));
        } else if let Some(pwm) = fan.pwm {
            let percent = (f64::from(pwm.min(255)) * 100.0 / 255.0).round();
            metrics
                .push(Metric::new(metric_ids::FAN, "Fan", percent, Some("%")).range(100.0, None));
        }
    }
    if let Some(uptime) = status.boot.as_ref().and_then(|boot| boot.uptime_s) {
        metrics.push(Metric::new(
            metric_ids::UPTIME,
            "Uptime",
            uptime as f64,
            Some("s"),
        ));
    }
    metrics
}

pub(crate) struct BoardStatus {
    pub(crate) http: reqwest::Client,
    pub(crate) status_url: String,
    pub(crate) events_url: Option<String>,
}

impl BoardStatus {
    pub(crate) async fn read(&self) -> Result<DeviceStatus, DriverError> {
        Ok(parse_status(
            &get_json(&self.http, &self.status_url).await?,
            now_s(),
        ))
    }
}

#[async_trait]
impl StatusCapability for BoardStatus {
    async fn status(&self, _device: &Identity) -> Result<DeviceStatus, DriverError> {
        self.read().await
    }

    async fn events(
        &self,
        device: &Identity,
        since: Option<i64>,
        limit: usize,
    ) -> Result<Vec<DeviceEvent>, DriverError> {
        let query = EventQuery {
            since,
            after_seq: None,
            limit,
        };
        Ok(self.event_page(device, query).await?.events)
    }

    async fn event_page(
        &self,
        _device: &Identity,
        query: EventQuery,
    ) -> Result<EventPage, DriverError> {
        let Some(url) = &self.events_url else {
            return Ok(EventPage::default());
        };
        let separator = if url.contains('?') { '&' } else { '?' };
        let limit = query.limit.clamp(1, 2000);
        // A board from before seq ignores after_seq and answers by time,
        // without `seq`: the caller then keeps asking by time.
        let url = match query.after_seq {
            Some(after) => format!("{url}{separator}after_seq={after}&limit={limit}"),
            None => format!(
                "{url}{separator}since={}&limit={limit}",
                query.since.unwrap_or(0).max(0)
            ),
        };
        Ok(parse_event_page(get_json(&self.http, &url).await?))
    }
}

/// Telemetry from the status endpoint.
pub(crate) struct StatusTelemetry(pub(crate) std::sync::Arc<BoardStatus>);

#[async_trait]
impl TelemetryCapability for StatusTelemetry {
    async fn read(&self, _device: &Identity) -> Result<Vec<Metric>, DriverError> {
        Ok(status_metrics(&self.0.read().await?))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> Value {
        json!({
            "version": 1, "time": 1_000,
            "boot": { "id": "b-1", "count": 12, "slot": "A", "kernel": "7.2.9", "previous_clean": false,
                      "at": 900, "uptime_s": 100 },
            "failed_units": ["foo.service", 3],
            "temperatures": [{ "id": "gpu", "celsius": 40.0 }, { "id": "cpu-thermal", "celsius": 54.3 }, { "id": 1 }],
            "fan": { "state": 2, "max_state": 4, "pwm": 212, "rpm": null },
            "update": { "state": "staging", "slot_active": "A", "slot_staged": "", "version_active": "1.0",
                        "version_staged": "", "progress": 420, "error": "", "version_previous": null,
                        "started_by": "orion" },
            "clock": { "time": 1_000, "ntp_synchronized": false },
            "drift": { "checked_at": 990, "slot": "A", "root_read_only": true, "baseline": "stage", "count": 1,
                       "flags": ["cmdline"],
                       "items": [{ "area": "boot", "path": "cmdline.txt", "change": "changed", "sha256": "ab" }] }
        })
    }

    #[test]
    fn the_status_document_parses() {
        let status = parse_status(&sample(), 1_030);
        assert_eq!(status.clock_offset_s, Some(-30));
        assert_eq!(status.ntp_synchronized, Some(false));
        let boot = status.boot.unwrap();
        assert_eq!((boot.count, boot.previous_clean), (Some(12), Some(false)));
        // A list with a bad entry is unreadable as a whole: unknown.
        assert_eq!(status.failed_units, None);
        assert_eq!(status.temperatures.len(), 2);
        let update = status.update.unwrap();
        assert_eq!(update.state, "staging");
        assert_eq!(update.slot_staged, None);
        assert_eq!(update.error, None);
        assert_eq!(update.started_by, Some(EventSource::Orion));
        assert!(update.busy());
        let drift = status.drift.unwrap();
        assert_eq!(drift.count, 1);
        assert_eq!(drift.items[0].path, "cmdline.txt");
        assert_eq!(status.fan.unwrap().pwm, Some(212));
    }

    #[test]
    fn hardware_parses_with_nulls_and_units() {
        let status = parse_status(
            &json!({ "time": 5, "hardware": {
                "at": 1_791_590_400, "future": "ignored",
                "devices": [
                    { "id": "imu", "class": "imu", "model": "bmi088", "status": "available",
                      "readings": [ { "name": "accel_x", "value": 0.12, "unit": "m/s²" },
                                    { "name": "gyro_z", "value": null, "unit": "" } ],
                      "controls": [] },
                    { "id": "fan", "class": "fan", "model": "pwmfan", "status": "available",
                      "readings": [ { "name": "duty", "value": 0.83, "unit": "" } ],
                      "controls": [ "duty" ] },
                    { "id": "gone", "class": "fan", "model": "pwmfan", "status": "missing",
                      "readings": [], "controls": [] }
                ]
            } }),
            5,
        );
        let hardware = status.hardware.expect("hardware");
        assert_eq!(hardware.at, 1_791_590_400);
        assert_eq!(hardware.devices.len(), 3);
        let imu = &hardware.devices[0];
        assert_eq!((imu.id.as_str(), imu.model.as_str()), ("imu", "bmi088"));
        assert_eq!(imu.readings[0].value, Some(0.12));
        assert_eq!(imu.readings[0].unit, "m/s²");
        assert_eq!(imu.readings[1].value, None);
        assert_eq!(
            hardware.devices[1].controls,
            [atlas_driver::HardwareControl::named("duty")]
        );
        assert!(hardware.devices[2].readings.is_empty());
    }

    #[test]
    fn hardware_null_or_malformed_is_unknown() {
        let null = parse_status(&json!({ "time": 5, "hardware": null }), 5);
        assert_eq!(null.hardware, None);
        let absent = parse_status(&json!({ "time": 5 }), 5);
        assert_eq!(absent.hardware, None);
        // A bad part leaves hardware out; the rest of the status still reads.
        let malformed = parse_status(
            &json!({ "time": 5, "fan": { "pwm": 9 }, "hardware": { "at": 1, "devices": "imu" } }),
            5,
        );
        assert_eq!(malformed.hardware, None);
        assert_eq!(malformed.time, Some(5));
        assert_eq!(malformed.fan.unwrap().pwm, Some(9));
    }

    #[test]
    fn missing_parts_are_unknown() {
        let status = parse_status(&json!({ "time": 5, "update": null, "drift": "x" }), 5);
        assert_eq!(status.clock_offset_s, Some(0));
        assert!(status.update.is_none() && status.drift.is_none() && status.boot.is_none());
        assert_eq!(parse_status(&json!([]), 5), DeviceStatus::default());
    }

    #[test]
    fn readings_come_from_the_cpu_zone_and_the_fan() {
        let metrics = status_metrics(&parse_status(&sample(), 1_000));
        let ids: Vec<&str> = metrics.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["temp", "fan", "uptime"]);
        assert_eq!(metrics[0].value, 54.3);
        assert_eq!(
            (metrics[1].value, metrics[1].unit.as_deref()),
            (83.0, Some("%"))
        );
    }

    #[test]
    fn events_parse_and_bad_ones_are_skipped() {
        let events = parse_events(json!({ "time": 1, "boot_id": "b", "events": [
            { "t": 10, "boot_id": "b", "kind": "update.stage", "source": "atlas", "message": "staging", "data": { "slot": "B" } },
            { "t": "x", "kind": "broken" },
            { "t": 11, "boot_id": "b", "kind": "", "source": "local", "message": "no kind" },
            { "t": 12, "boot_id": "b", "kind": "boot", "source": "local", "message": "boot 3", "data": {} }
        ]}));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].source, EventSource::Atlas);
        assert_eq!(events[0].data["slot"], "B");
        assert_eq!(parse_events(json!("nope")), Vec::new());
    }

    #[test]
    fn a_page_has_the_boards_clock_boot_and_newest_seq() {
        let page = parse_event_page(json!({ "time": 1200, "boot_id": "b2", "seq": 4, "events": [
            { "t": 1300, "boot_id": "b2", "seq": 4, "uptime_s": 9, "kind": "update.confirmed", "source": "local", "message": "kept" }
        ]}));
        assert_eq!(
            (page.time, page.boot_id.as_deref(), page.newest_seq),
            (Some(1200), Some("b2"), Some(4))
        );
        assert_eq!(
            (page.events[0].seq, page.events[0].uptime_s),
            (Some(4), Some(9))
        );
        // A board from before seq.
        let old = parse_event_page(json!({ "time": 1, "boot_id": "", "events": [] }));
        assert_eq!((old.boot_id, old.newest_seq), (None, None));
    }
}
