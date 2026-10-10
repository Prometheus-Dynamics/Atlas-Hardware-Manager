//! A board's state as Orion knows it: the node's host facts (boot id,
//! kernel), its `host.*` readings (temperatures, uptime) and the `update.*`
//! keys the board's agent publishes. Orion keeps no event log of the board,
//! so there are no events; a board whose package serves `/status` reports
//! the rest itself, and that wins (Orion only fills in what is missing).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use atlas_driver::{
    BootInfo, DeviceStatus, DriverError, EventSource, HardwareDevice, HardwareSnapshot, Identity,
    StatusCapability, Temperature, UpdateState,
};
use futures::future::join_all;
use orion_control_plane::{
    NodeHostFacts, StatusEntry, StatusQuery, StatusSubject, TypedConfigValue, update_action,
};
use orion_core::NodeId;

use crate::hardware;
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
            ..BootInfo::default()
        }),
        temperatures,
        update,
        hardware: None,
        ..DeviceStatus::default()
    }
}

pub(crate) struct OrionStatus {
    pub(crate) transport: Arc<dyn OrionTransport>,
    pub(crate) node: NodeId,
}

#[async_trait]
impl StatusCapability for OrionStatus {
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
    fn a_node_without_an_agent_has_no_update() {
        assert!(status_from(None, &[]).update.is_none());
    }
}
