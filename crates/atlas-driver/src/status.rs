//! What a running device is doing and how it is, and what happened to it.
//!
//! A board reports its state ([`DeviceStatus`]: boot, failed services,
//! temperatures and fan, its update, clock and drift) and an event log
//! ([`DeviceEvent`]) that says who did what: Atlas over SSH, Orion through
//! the board's agent, or someone on the board itself. Atlas merges the
//! events into the device's history.
//!
//! Every field is optional: a device reports what it knows, and a driver for
//! a management agent such as Orion fills only some of it.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{DriverError, Identity};

/// Who asked for something that happened on a device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventSource {
    /// Atlas, over SSH.
    Atlas,
    /// Orion, through the board's agent.
    Orion,
    /// Someone (or something) on the board itself.
    Local,
    #[default]
    Unknown,
}

impl EventSource {
    pub fn parse(text: &str) -> Self {
        match text {
            "atlas" => Self::Atlas,
            "orion" => Self::Orion,
            "local" => Self::Local,
            _ => Self::Unknown,
        }
    }
}

impl<'de> Deserialize<'de> for EventSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<String>::deserialize(deserializer)?
            .as_deref()
            .map_or(Self::Unknown, Self::parse))
    }
}

/// One line of a device's event log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEvent {
    /// The device's clock, Unix seconds (a board without an RTC may be off).
    pub t: i64,
    #[serde(default)]
    pub boot_id: String,
    /// Dotted, for example `update.staged`, `boot`, `clock.set`.
    pub kind: String,
    #[serde(default)]
    pub source: EventSource,
    #[serde(default)]
    pub message: String,
    #[serde(default, deserialize_with = "lenient_map")]
    pub data: BTreeMap<String, String>,
    /// The device's number for it: events in the order they were written,
    /// across boots, whatever its clock did. `None` from a device that
    /// doesn't number its events.
    #[serde(default)]
    pub seq: Option<u64>,
    /// The device's uptime when it happened (seconds).
    #[serde(default)]
    pub uptime_s: Option<u64>,
    /// When it happened by this computer's clock (Unix ms), when Atlas knew
    /// how far the device's clock was off in that boot; `t` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

impl DeviceEvent {
    /// Events are the same when their boot and seq are, or for events
    /// without a seq, their boot, time, kind and message.
    pub fn same_as(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (self.seq, other.seq) {
            return a == b && self.boot_id == other.boot_id;
        }
        self.t == other.t
            && self.boot_id == other.boot_id
            && self.kind == other.kind
            && self.message == other.message
    }

    /// When it happened by this computer's clock, as well as Atlas knows.
    pub fn when_ms(&self) -> u64 {
        self.at_ms
            .unwrap_or_else(|| u64::try_from(self.t).unwrap_or(0).saturating_mul(1000))
    }

    /// How it reads: `"error"`, `"warning"`, `"success"` or `"info"`.
    pub fn severity(&self) -> &'static str {
        let kind = self.kind.as_str();
        if kind.ends_with("failed") || kind == "update.link-fallback" {
            "error"
        } else if kind == "update.rolled-back"
            || kind == "update.apply-interrupted"
            || kind == "update.link-bad"
            || kind == "usb-boot"
            || (kind == "boot"
                && self.data.get("previous_clean").map(String::as_str) == Some("false"))
            || (kind == "selftest" && self.data.get("ok").map(String::as_str) == Some("false"))
        {
            "warning"
        } else if kind == "update.confirmed"
            || kind == "update.staged"
            || (kind == "selftest" && self.data.get("ok").map(String::as_str) == Some("true"))
        {
            "success"
        } else {
            "info"
        }
    }
}

/// Strings stay strings; numbers and booleans become their text; anything
/// else is dropped.
fn lenient_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    let raw = Option::<BTreeMap<String, serde_json::Value>>::deserialize(deserializer)?;
    Ok(raw
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, value)| match value {
            serde_json::Value::String(text) => Some((key, text)),
            serde_json::Value::Number(n) => Some((key, n.to_string())),
            serde_json::Value::Bool(b) => Some((key, b.to_string())),
            _ => None,
        })
        .collect())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BootInfo {
    #[serde(default)]
    pub id: Option<String>,
    /// Boots since the board's data was made.
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub kernel: Option<String>,
    #[serde(default)]
    pub uptime_s: Option<u64>,
    /// Whether the boot before this one shut down cleanly; `None` when the
    /// board can't tell.
    #[serde(default)]
    pub previous_clean: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Temperature {
    pub id: String,
    pub celsius: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanState {
    #[serde(default)]
    pub state: Option<u32>,
    #[serde(default)]
    pub max_state: Option<u32>,
    /// Duty, 0-255.
    #[serde(default)]
    pub pwm: Option<u32>,
    #[serde(default)]
    pub rpm: Option<u32>,
}

/// The device's own update state (the A/B writer's `update status`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    /// `idle`, `staging`, `staged`, `rebooting`, `trying`, `confirmed`,
    /// `rolled-back`, `cancelled`, `error`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub slot_active: Option<String>,
    #[serde(default)]
    pub slot_staged: Option<String>,
    #[serde(default)]
    pub version_active: Option<String>,
    #[serde(default)]
    pub version_staged: Option<String>,
    /// What a rollback would go back to, when there is something.
    #[serde(default)]
    pub version_previous: Option<String>,
    /// Per mille of the current step.
    #[serde(default)]
    pub progress: u32,
    #[serde(default)]
    pub error: Option<String>,
    /// Who started the update in progress or last finished.
    #[serde(default)]
    pub started_by: Option<EventSource>,
}

impl UpdateState {
    /// Something is under way: a stage, a restart or a trial boot.
    pub fn busy(&self) -> bool {
        matches!(self.state.as_str(), "staging" | "rebooting" | "trying")
    }
}

/// One file that differs from what the device expects, or an override.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftItem {
    /// `boot` (the running boot slot), `root`, `etc` or `data` (overrides).
    pub area: String,
    pub path: String,
    /// `changed`, `added`, `missing`, or `present` (an override).
    pub change: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drift {
    /// When the device checked (its clock, Unix seconds).
    #[serde(default)]
    pub checked_at: Option<i64>,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub root_read_only: Option<bool>,
    /// What the boot files were compared with: `stage`, `first-seen`, `none`.
    #[serde(default)]
    pub baseline: Option<String>,
    /// Items that count as drift (overrides in /etc on a read-only root
    /// don't).
    #[serde(default)]
    pub count: u32,
    /// Notable changes: `cmdline`, `config`, `sshd_config`, `update_env`.
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub items: Vec<DriftItem>,
}

/// One reading of a device's sensor, with its unit (empty when it has none).
/// `value` is `None` when the device can't tell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HardwareReading {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub unit: String,
}

/// One control a board device accepts, with what the source knows of it: its
/// value now and its range, in `unit` (empty when it has none).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HardwareControl {
    pub name: String,
    pub value: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: String,
}

impl HardwareControl {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

/// A board lists its controls by name; Orion describes them. Both read.
impl<'de> Deserialize<'de> for HardwareControl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Name(String),
            Full {
                #[serde(default)]
                name: String,
                #[serde(default)]
                value: Option<f64>,
                #[serde(default)]
                min: Option<f64>,
                #[serde(default)]
                max: Option<f64>,
                #[serde(default)]
                unit: String,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Name(name) => Self::named(name),
            Wire::Full {
                name,
                value,
                min,
                max,
                unit,
            } => Self {
                name,
                value,
                min,
                max,
                unit,
            },
        })
    }
}

/// One device on a board's hardware bus, with its latest readings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HardwareDevice {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub model: String,
    /// `available`, `degraded`, `faulted` or `missing`.
    #[serde(default)]
    pub status: String,
    /// Why it isn't available, when the source says (Orion's `reason`).
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub readings: Vec<HardwareReading>,
    /// The controls the device accepts.
    #[serde(default)]
    pub controls: Vec<HardwareControl>,
}

/// What a board's devices read at one moment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HardwareSnapshot {
    /// When the snapshot was taken (the board's clock, Unix seconds).
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub devices: Vec<HardwareDevice>,
}

/// A device's state now. Absent parts are unknown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceStatus {
    /// The device's clock when it answered (Unix seconds).
    #[serde(default)]
    pub time: Option<i64>,
    #[serde(default)]
    pub boot: Option<BootInfo>,
    /// Failed system services, by unit name; `None` when the device can't
    /// tell (an empty list means none failed).
    #[serde(default)]
    pub failed_units: Option<Vec<String>>,
    #[serde(default)]
    pub temperatures: Vec<Temperature>,
    #[serde(default)]
    pub fan: Option<FanState>,
    #[serde(default)]
    pub update: Option<UpdateState>,
    /// How far the device's clock is from this computer's, in seconds
    /// (negative: behind).
    #[serde(default)]
    pub clock_offset_s: Option<i64>,
    #[serde(default)]
    pub ntp_synchronized: Option<bool>,
    #[serde(default)]
    pub drift: Option<Drift>,
    /// The board's sensors and devices; `None` when unknown (no hardware
    /// service, or a stale snapshot).
    #[serde(default)]
    pub hardware: Option<HardwareSnapshot>,
}

/// Which events to fetch: those after a seq (from a device that numbers its
/// events), else those at or after a time; at most `limit`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventQuery {
    pub since: Option<i64>,
    pub after_seq: Option<u64>,
    pub limit: usize,
}

/// A page of a device's event log, with what the device said about itself
/// when it answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventPage {
    /// Oldest first: by seq after a seq, else by time.
    pub events: Vec<DeviceEvent>,
    /// The device's clock (Unix s) and boot when it answered, to place this
    /// boot's events in time.
    pub time: Option<i64>,
    pub boot_id: Option<String>,
    /// The newest seq the device has written; `None` when it doesn't number
    /// its events.
    pub newest_seq: Option<u64>,
}

/// What a device's push channel said: something changed, so re-read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusPush {
    /// Connected (again): the device's boot and newest event seq.
    Hello {
        boot_id: String,
        newest_seq: Option<u64>,
    },
    /// A new event in its log.
    Event(Box<DeviceEvent>),
    /// Its update state changed; the new state (`staging`, `trying`, …).
    Update { state: String },
}

/// Where a push channel's messages go.
pub type PushSink = std::sync::Arc<dyn Fn(StatusPush) + Send + Sync>;

/// A device that reports its state and, optionally, an event log.
#[async_trait]
pub trait StatusCapability: Send + Sync {
    /// Holds the device's push channel open, passing each message to `sink`,
    /// until it ends (`Ok(true)`: the device went or closed it; reconnect
    /// later) or can't be opened (an error). `Ok(false)` at once: no push
    /// channel; the caller polls.
    async fn watch(&self, _device: &Identity, _sink: PushSink) -> Result<bool, DriverError> {
        Ok(false)
    }

    /// Streams `wanted` (board device ids with periods in ms) live into
    /// `sink` until the stream ends (`Ok(true)`) or the caller drops this;
    /// `Ok(false)` at once: no live stream (the snapshot in the status is
    /// all there is).
    async fn stream_hardware(
        &self,
        _device: &Identity,
        _wanted: &[(String, u32)],
        _sink: crate::FrameSink,
    ) -> Result<bool, DriverError> {
        Ok(false)
    }

    async fn status(&self, device: &Identity) -> Result<DeviceStatus, DriverError>;

    /// A page of the event log. By default, [`events`](Self::events) by
    /// time: a device that numbers its events answers `after_seq` itself.
    async fn event_page(
        &self,
        device: &Identity,
        query: EventQuery,
    ) -> Result<EventPage, DriverError> {
        Ok(EventPage {
            events: self.events(device, query.since, query.limit).await?,
            ..EventPage::default()
        })
    }

    /// Events with `t >= since` (device clock), at most `limit`, oldest
    /// first. A device without an event log has none.
    async fn events(
        &self,
        _device: &Identity,
        _since: Option<i64>,
        _limit: usize,
    ) -> Result<Vec<DeviceEvent>, DriverError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_read_leniently() {
        let event: DeviceEvent = serde_json::from_str(
            r#"{"t":5,"boot_id":"b","kind":"boot","source":"martian","message":"m","data":{"count":3,"ok":true,"x":[1],"slot":"A"}}"#,
        )
        .unwrap();
        assert_eq!(event.source, EventSource::Unknown);
        assert_eq!(event.data["count"], "3");
        assert_eq!(event.data["ok"], "true");
        assert!(!event.data.contains_key("x"));
        let bare: DeviceEvent = serde_json::from_str(r#"{"t":1,"kind":"x","data":null}"#).unwrap();
        assert_eq!(bare.source, EventSource::Unknown);
        assert!(bare.data.is_empty());
    }

    #[test]
    fn controls_read_as_names_or_in_full() {
        let device: HardwareDevice = serde_json::from_str(
            r#"{"id":"fan","controls":["duty",{"name":"speed","value":0.5,"min":0,"max":1,"unit":""}]}"#,
        )
        .unwrap();
        assert_eq!(device.controls[0], HardwareControl::named("duty"));
        assert_eq!(device.controls[1].value, Some(0.5));
        assert_eq!(device.controls[1].max, Some(1.0));
    }

    #[test]
    fn numbered_events_are_the_same_by_boot_and_seq() {
        let event: DeviceEvent = serde_json::from_str(
            r#"{"t":5,"boot_id":"b","seq":7,"uptime_s":3,"kind":"boot","message":"boot 2"}"#,
        )
        .unwrap();
        assert_eq!(
            (event.seq, event.uptime_s, event.at_ms),
            (Some(7), Some(3), None)
        );
        assert_eq!(event.when_ms(), 5_000);
        // Same boot and seq: the same event, whatever the clock said.
        let mut again = event.clone();
        again.t = 9;
        assert!(event.same_as(&again));
        let mut next = event.clone();
        next.seq = Some(8);
        assert!(
            !event.same_as(&next),
            "the same second and words, another seq"
        );
        let mut corrected = event.clone();
        corrected.at_ms = Some(42_000);
        assert_eq!(corrected.when_ms(), 42_000);
    }

    #[test]
    fn severity_follows_the_kind() {
        let event = |kind: &str, data: &[(&str, &str)]| DeviceEvent {
            t: 0,
            boot_id: String::new(),
            kind: kind.into(),
            source: EventSource::Local,
            message: String::new(),
            data: data
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            seq: None,
            uptime_s: None,
            at_ms: None,
        };
        assert_eq!(event("update.trial-failed", &[]).severity(), "error");
        assert_eq!(event("update.confirmed", &[]).severity(), "success");
        assert_eq!(
            event("boot", &[("previous_clean", "false")]).severity(),
            "warning"
        );
        assert_eq!(
            event("boot", &[("previous_clean", "true")]).severity(),
            "info"
        );
        assert_eq!(event("selftest", &[("ok", "false")]).severity(), "warning");
    }
}
