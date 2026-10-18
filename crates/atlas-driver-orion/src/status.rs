//! A board's state as Orion knows it: the node's host facts (boot id,
//! kernel), its `host.*` readings (temperatures, uptime) and the `update.*`
//! keys the board's agent publishes. Orion keeps no event log of the board,
//! so there are no events; a board whose package serves `/status` reports
//! the rest itself, and that wins (Orion only fills in what is missing).
//!
//! The push channel is the board's own node: a watch on its `update.*` keys
//! over a push session on its `orion+tcp` listener (as [`LiveStreams`]
//! connects), so a board seen only through Orion is pushed to, not polled,
//! like one with board-stream. board-agent names its newest event there
//! (`update.event_seq`, `_kind`, `_t`) and its update state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use atlas_driver::{
    BootInfo, DeviceEvent, DeviceStatus, DriverError, EventSource, HardwareDevice,
    HardwareSnapshot, Identity, KernelRecord, PushSink, StatusCapability, StatusPush, Temperature,
    UpdateState,
};
use futures::future::join_all;
use orion_control_plane::{
    ClientEventKind, NodeHostFacts, StatusChange, StatusEntry, StatusQuery, StatusSubject,
    TypedConfigValue, update_action,
};
use orion_core::NodeId;

use crate::hardware;
use crate::live::LiveStreams;
use crate::metrics::number;
use crate::transport::OrionTransport;

pub(crate) fn text(value: &TypedConfigValue) -> Option<String> {
    match value {
        TypedConfigValue::String(text) => Some(text.clone()),
        TypedConfigValue::UInt(n) => Some(n.to_string()),
        TypedConfigValue::Int(n) => Some(n.to_string()),
        TypedConfigValue::Bool(b) => Some(b.to_string()),
        TypedConfigValue::F64(x) => Some(x.to_string()),
        TypedConfigValue::Bytes(_) => None,
    }
}

/// The status from a node's host facts and its status entries.
pub(crate) fn status_from(host: Option<&NodeHostFacts>, entries: &[StatusEntry]) -> DeviceStatus {
    let keys: HashMap<&str, &TypedConfigValue> = entries
        .iter()
        .map(|entry| (entry.key.as_str(), &entry.value))
        .collect();
    let get = |key: &str| {
        keys.get(key)
            .and_then(|value| text(value))
            .filter(|value| !value.is_empty())
    };
    let mut temperatures: Vec<Temperature> = entries
        .iter()
        .filter_map(|entry| {
            let sensor = entry.key.strip_prefix("host.temperature.")?;
            Some(Temperature {
                id: sensor.to_string(),
                celsius: (number(&entry.value)? / 100.0).round() / 10.0,
            })
        })
        .collect();
    temperatures.sort_by(|a, b| a.id.cmp(&b.id));
    let update = get(update_action::KEY_STATE).map(|state| UpdateState {
        state,
        slot_active: get(update_action::KEY_SLOT_ACTIVE),
        slot_staged: get(update_action::KEY_SLOT_STAGED),
        version_active: get(update_action::KEY_VERSION_ACTIVE),
        version_staged: get(update_action::KEY_VERSION_STAGED),
        // board-agent's own keys (crates/board-agent, agent.rs).
        version_previous: get("update.version_previous"),
        progress: keys
            .get(update_action::KEY_PROGRESS)
            .and_then(|value| number(value))
            .map_or(0, |permille| permille.clamp(0.0, 1000.0) as u32),
        error: get(update_action::KEY_ERROR),
        started_by: get("update.started_by").map(|who| EventSource::parse(&who)),
    });
    DeviceStatus {
        boot: host.map(|host| BootInfo {
            id: host.boot_id.clone(),
            kernel: host.kernel_release.clone(),
            uptime_s: keys
                .get("host.uptime_seconds")
                .and_then(|value| number(value))
                .map(|seconds| seconds as u64),
            slot: update
                .as_ref()
                .and_then(|update| update.slot_active.clone()),
            // board-agent's boot record (published.rs).
            count: keys
                .get("update.boot_count")
                .and_then(|value| number(value))
                .map(|count| count as u64)
                .filter(|count| *count > 0),
            reset: get("update.boot_reset"),
            previous_clean: get("update.boot_previous_clean").and_then(|clean| clean.parse().ok()),
            ..BootInfo::default()
        }),
        // Only this boot's: what it found of the boot before.
        pstore: host
            .and_then(|host| host.boot_id.clone())
            .zip(get("update.boot_kernel_log"))
            .map(|(boot, log)| {
                vec![KernelRecord {
                    boot,
                    panic: log == "panic",
                    files: Vec::new(),
                }]
            }),
        temperatures,
        update,
        hardware: None,
        ..DeviceStatus::default()
    }
}

/// board-agent's newest event (crates/board-agent, published.rs).
const KEY_EVENT_SEQ: &str = "update.event_seq";
const KEY_EVENT_KIND: &str = "update.event_kind";
const KEY_EVENT_T: &str = "update.event_t";

/// The `update.*` keys a watch has seen, and what their changes push.
#[derive(Default)]
struct WatchedKeys {
    keys: HashMap<String, TypedConfigValue>,
}

impl WatchedKeys {
    fn text(&self, key: &str) -> Option<String> {
        self.keys
            .get(key)
            .and_then(text)
            .filter(|value| !value.is_empty())
    }

    fn seq(&self) -> Option<u64> {
        match self.keys.get(KEY_EVENT_SEQ)? {
            TypedConfigValue::UInt(seq) => Some(*seq),
            TypedConfigValue::Int(seq) => u64::try_from(*seq).ok(),
            _ => None,
        }
    }

    /// What `change` pushes: `Hello` for the watch's first (every key), then
    /// an `Event` when the newest event's seq moves and an `Update` when the
    /// update state does.
    fn apply(&mut self, change: &StatusChange) -> Vec<StatusPush> {
        let (seq, state) = (self.seq(), self.text(update_action::KEY_STATE));
        if change.bootstrap {
            self.keys.clear();
        }
        for entry in &change.updated {
            self.keys.insert(entry.key.clone(), entry.value.clone());
        }
        for key in &change.expired {
            self.keys.remove(&key.key);
        }
        let boot_id = self.text(update_action::KEY_BOOT_ID).unwrap_or_default();
        if change.bootstrap {
            return vec![StatusPush::Hello {
                boot_id,
                newest_seq: self.seq(),
            }];
        }
        let mut pushes = Vec::new();
        if let Some(newest) = self.seq().filter(|newest| Some(*newest) != seq) {
            pushes.push(StatusPush::Event(Box::new(DeviceEvent {
                t: self
                    .keys
                    .get(KEY_EVENT_T)
                    .and_then(number)
                    .map_or(0, |t| t as i64),
                boot_id,
                kind: self.text(KEY_EVENT_KIND).unwrap_or_default(),
                source: EventSource::default(),
                message: String::new(),
                data: Default::default(),
                seq: Some(newest),
                uptime_s: None,
                at_ms: None,
            })));
        }
        if let Some(now) = self
            .text(update_action::KEY_STATE)
            .filter(|now| Some(now) != state.as_ref())
        {
            pushes.push(StatusPush::Update { state: now });
        }
        pushes
    }
}

pub(crate) struct OrionStatus {
    pub(crate) transport: Arc<dyn OrionTransport>,
    pub(crate) node: NodeId,
    /// Connects to boards' own nodes for the push channel; `None`: polled.
    pub(crate) live: Option<Arc<LiveStreams>>,
}

#[async_trait]
impl StatusCapability for OrionStatus {
    /// Watches the board's `update.*` keys on its own node. `Ok(false)`
    /// without a way to reach it (no address); an error when its node can't
    /// be reached, isn't this board's, refuses the watch or has no agent
    /// publishing (Atlas polls, and tries again less and less often).
    async fn watch(&self, device: &Identity, sink: PushSink) -> Result<bool, DriverError> {
        let address = device.address.trim();
        let Some(live) = &self.live else {
            return Ok(false);
        };
        if address.is_empty() || address.starts_with('/') {
            return Ok(false);
        }
        let url = LiveStreams::url_for(address);
        let operator = live.connect(&url).await?;
        if operator.node_id() != &self.node {
            return Err(DriverError::Unreachable(format!(
                "Orion at {url} is node {}, not {}",
                operator.node_id(),
                self.node
            )));
        }
        let unreachable = |error: orion_client::remote::RemoteError| {
            DriverError::Unreachable(format!("Orion at {url}: {error}"))
        };
        let mut session = operator.open_session("atlas").await.map_err(unreachable)?;
        session
            .watch_status(StatusQuery {
                subject: Some(StatusSubject::Node(self.node.clone())),
                key_prefix: Some("update.".into()),
            })
            .await
            .map_err(unreachable)?;
        let mut keys = WatchedKeys::default();
        while let Some(event) = session.next_event().await {
            let ClientEventKind::Status(change) = event.event else {
                continue;
            };
            if change.bootstrap && change.updated.is_empty() {
                return Err(DriverError::Unsupported(format!(
                    "no device agent publishes on node {}",
                    self.node
                )));
            }
            for push in keys.apply(&change) {
                sink(push);
            }
        }
        Ok(true)
    }

    async fn status(&self, _device: &Identity) -> Result<DeviceStatus, DriverError> {
        let host = self
            .transport
            .nodes()
            .await?
            .into_iter()
            .find(|record| record.node_id == self.node)
            .and_then(|record| record.host);
        let mut entries = Vec::new();
        for prefix in ["host.", "update."] {
            entries.extend(
                self.transport
                    .status(StatusQuery {
                        subject: Some(StatusSubject::Node(self.node.clone())),
                        key_prefix: Some(prefix.into()),
                    })
                    .await?,
            );
        }
        let mut status = status_from(host.as_ref(), &entries);
        status.hardware = self.hardware().await;
        Ok(status)
    }
}

impl OrionStatus {
    /// The board's devices: the `lemnos.device` resources whose provider is
    /// on this node, each with its own status lane. `None` when there are
    /// none, or when Orion can't say (a device whose status fails is left out).
    async fn hardware(&self) -> Option<HardwareSnapshot> {
        let resources = self.transport.resources().await.ok()?;
        let transport = self.transport.as_ref();
        let queries = resources
            .into_iter()
            .filter(|(record, node)| {
                record.resource_type.as_str() == hardware::DEVICE_TYPE
                    && node.as_ref() == Some(&self.node)
            })
            .map(|(record, _)| async move {
                let query = StatusQuery {
                    subject: Some(StatusSubject::Resource(record.resource_id.clone())),
                    key_prefix: None,
                };
                let entries = transport.status(query).await.ok()?;
                Some(hardware::device(&record, &entries))
            });
        let devices: Vec<HardwareDevice> = join_all(queries).await.into_iter().flatten().collect();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0));
        hardware::snapshot(devices, now)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn entry(key: &str, value: TypedConfigValue) -> StatusEntry {
        StatusEntry::new(StatusSubject::Node(NodeId::new("raze-1")), key, value)
    }

    #[test]
    fn update_keys_and_host_facts_map_to_the_status() {
        let host = NodeHostFacts {
            boot_id: Some("b-2".into()),
            kernel_release: Some("7.2.9".into()),
            ..NodeHostFacts::default()
        };
        let status = status_from(
            Some(&host),
            &[
                entry("host.temperature.cpu", TypedConfigValue::UInt(54_321)),
                entry("host.uptime_seconds", TypedConfigValue::UInt(90)),
                entry("update.state", TypedConfigValue::String("staging".into())),
                entry("update.slot_active", TypedConfigValue::String("A".into())),
                entry(
                    "update.version_staged",
                    TypedConfigValue::String(String::new()),
                ),
                entry("update.progress", TypedConfigValue::UInt(420)),
            ],
        );
        let boot = status.boot.unwrap();
        assert_eq!(boot.id.as_deref(), Some("b-2"));
        assert_eq!(boot.uptime_s, Some(90));
        assert_eq!(boot.slot.as_deref(), Some("A"));
        assert_eq!(status.temperatures[0].celsius, 54.3);
        let update = status.update.unwrap();
        assert_eq!((update.state.as_str(), update.progress), ("staging", 420));
        assert_eq!(update.version_staged, None);
        assert!(status.drift.is_none());
    }

    #[test]
    fn the_boot_record_maps_to_the_boot() {
        let host = NodeHostFacts {
            boot_id: Some("b-2".into()),
            ..NodeHostFacts::default()
        };
        let status = status_from(
            Some(&host),
            &[
                entry("update.boot_count", TypedConfigValue::UInt(7)),
                entry(
                    "update.boot_reset",
                    TypedConfigValue::String("power-on".into()),
                ),
                entry(
                    "update.boot_previous_clean",
                    TypedConfigValue::String("false".into()),
                ),
                entry(
                    "update.boot_kernel_log",
                    TypedConfigValue::String("panic".into()),
                ),
            ],
        );
        let boot = status.boot.unwrap();
        assert_eq!(boot.count, Some(7));
        assert_eq!(boot.reset.as_deref(), Some("power-on"));
        assert_eq!(boot.previous_clean, Some(false));
        let pstore = status.pstore.unwrap();
        assert_eq!((pstore[0].boot.as_str(), pstore[0].panic), ("b-2", true));
        let quiet = status_from(
            Some(&host),
            &[entry(
                "update.boot_kernel_log",
                TypedConfigValue::String(String::new()),
            )],
        );
        assert_eq!(quiet.pstore, None);
        assert_eq!(quiet.boot.unwrap().previous_clean, None);
    }

    fn change(bootstrap: bool, keys: &[(&str, TypedConfigValue)]) -> StatusChange {
        StatusChange {
            bootstrap,
            updated: keys
                .iter()
                .map(|(key, value)| entry(key, value.clone()))
                .collect(),
            expired: Vec::new(),
        }
    }

    #[test]
    fn a_new_event_seq_or_update_state_is_pushed() {
        let mut keys = WatchedKeys::default();
        let state = |s: &str| TypedConfigValue::String(s.into());
        assert_eq!(
            keys.apply(&change(true, &[("update.state", state("idle"))])),
            vec![StatusPush::Hello {
                boot_id: String::new(),
                newest_seq: None,
            }]
        );
        assert!(
            keys.apply(&change(false, &[("update.state", state("idle"))]))
                .is_empty(),
            "the same state again"
        );
        let pushes = keys.apply(&change(
            false,
            &[
                ("update.event_seq", TypedConfigValue::UInt(1)),
                ("update.event_kind", state("clock.set")),
            ],
        ));
        let [StatusPush::Event(event)] = pushes.as_slice() else {
            panic!("{pushes:?}");
        };
        assert_eq!((event.seq, event.kind.as_str()), (Some(1), "clock.set"));
        assert_eq!(
            keys.apply(&change(false, &[("update.state", state("trying"))])),
            vec![StatusPush::Update {
                state: "trying".into()
            }]
        );
    }

    #[test]
    fn a_node_without_an_agent_has_no_update() {
        assert!(status_from(None, &[]).update.is_none());
    }
}
