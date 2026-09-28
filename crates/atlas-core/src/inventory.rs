use std::collections::{BTreeMap, BTreeSet};

use atlas_driver::{CapabilityKind, DeviceKey, Identity, LinkKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Presence {
    Online,
    Offline,
}

/// Everything Atlas knows about one device, online or not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub key: DeviceKey,
    /// The identity from the most recent sighting.
    pub identity: Identity,
    pub link_kind: LinkKind,
    pub capabilities: Vec<CapabilityKind>,
    pub presence: Presence,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    /// A name the user gave the device in Atlas.
    #[serde(default)]
    pub label: Option<String>,
    /// The robot profile this device belongs to.
    #[serde(default)]
    pub robot: Option<String>,
}

impl DeviceRecord {
    /// The user's label, then the device's own name, then its key.
    pub fn display_name(&self) -> String {
        self.label
            .clone()
            .or_else(|| self.identity.name.clone())
            .unwrap_or_else(|| self.key.to_string())
    }

    fn same_details(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.link_kind == other.link_kind
            && self.capabilities == other.capabilities
            && self.presence == other.presence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Upsert {
    New,
    Changed,
    Unchanged,
}

/// Preference when one device is reachable over several links in one scan.
/// Lower is better: direct fast links first, recovery paths last.
pub(crate) fn link_rank(kind: &LinkKind) -> u8 {
    match kind {
        LinkKind::UsbNetwork => 0,
        LinkKind::Ethernet => 1,
        LinkKind::UsbSerial => 2,
        LinkKind::Gateway { .. } => 3,
        LinkKind::UsbBoot => 4,
        LinkKind::Simulated => 5,
    }
}

#[derive(Default)]
pub(crate) struct Inventory {
    records: BTreeMap<DeviceKey, DeviceRecord>,
}

impl Inventory {
    /// Restores saved records. Nothing is online until a scan sees it.
    pub(crate) fn from_records(records: Vec<DeviceRecord>) -> Self {
        let records = records
            .into_iter()
            .map(|mut record| {
                record.presence = Presence::Offline;
                (record.key.clone(), record)
            })
            .collect();
        Self { records }
    }

    pub(crate) fn upsert(
        &mut self,
        identity: Identity,
        link_kind: LinkKind,
        capabilities: Vec<CapabilityKind>,
        now_ms: u64,
    ) -> (Upsert, DeviceRecord) {
        let key = identity.key.clone();
        match self.records.get_mut(&key) {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.identity = identity;
                updated.link_kind = link_kind;
                updated.capabilities = capabilities;
                updated.presence = Presence::Online;
                updated.last_seen_ms = now_ms;
                let outcome = if existing.same_details(&updated) {
                    Upsert::Unchanged
                } else {
                    Upsert::Changed
                };
                *existing = updated.clone();
                (outcome, updated)
            }
            None => {
                let record = DeviceRecord {
                    key: key.clone(),
                    identity,
                    link_kind,
                    capabilities,
                    presence: Presence::Online,
                    first_seen_ms: now_ms,
                    last_seen_ms: now_ms,
                    label: None,
                    robot: None,
                };
                self.records.insert(key, record.clone());
                (Upsert::New, record)
            }
        }
    }

    /// Marks online devices missing from `seen` as offline and returns them.
    pub(crate) fn mark_offline_except(&mut self, seen: &BTreeSet<DeviceKey>) -> Vec<DeviceKey> {
        let mut gone = Vec::new();
        for (key, record) in &mut self.records {
            if record.presence == Presence::Online && !seen.contains(key) {
                record.presence = Presence::Offline;
                gone.push(key.clone());
            }
        }
        gone
    }

    pub(crate) fn get(&self, key: &DeviceKey) -> Option<&DeviceRecord> {
        self.records.get(key)
    }

    pub(crate) fn get_mut(&mut self, key: &DeviceKey) -> Option<&mut DeviceRecord> {
        self.records.get_mut(key)
    }

    /// Removes a device the user no longer wants remembered.
    pub(crate) fn remove(&mut self, key: &DeviceKey) -> Option<DeviceRecord> {
        self.records.remove(key)
    }

    pub(crate) fn all(&self) -> Vec<DeviceRecord> {
        self.records.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_driver::{DeviceMode, LinkId};

    use super::*;

    fn identity(serial: &str, version: &str) -> Identity {
        Identity {
            key: DeviceKey::new("helios", serial),
            model: "cm5".into(),
            mode: DeviceMode::Normal,
            versions: BTreeMap::from([("os".to_string(), version.to_string())]),
            name: None,
            link: LinkId("usbnet:0".into()),
            address: "10.0.0.2".into(),
        }
    }

    #[test]
    fn upsert_reports_new_unchanged_and_changed() {
        let mut inventory = Inventory::default();
        let caps = vec![CapabilityKind::Info];

        let (first, _) =
            inventory.upsert(identity("a", "1"), LinkKind::UsbNetwork, caps.clone(), 10);
        let (second, record) =
            inventory.upsert(identity("a", "1"), LinkKind::UsbNetwork, caps.clone(), 20);
        let (third, _) = inventory.upsert(identity("a", "2"), LinkKind::UsbNetwork, caps, 30);

        assert_eq!(first, Upsert::New);
        assert_eq!(second, Upsert::Unchanged);
        assert_eq!(record.first_seen_ms, 10);
        assert_eq!(record.last_seen_ms, 20);
        assert_eq!(third, Upsert::Changed);
    }

    #[test]
    fn devices_not_seen_go_offline_once() {
        let mut inventory = Inventory::default();
        inventory.upsert(identity("a", "1"), LinkKind::Ethernet, Vec::new(), 1);
        inventory.upsert(identity("b", "1"), LinkKind::Ethernet, Vec::new(), 1);

        let seen = BTreeSet::from([DeviceKey::new("helios", "a")]);
        assert_eq!(
            inventory.mark_offline_except(&seen),
            vec![DeviceKey::new("helios", "b")]
        );
        assert!(inventory.mark_offline_except(&seen).is_empty());
    }

    #[test]
    fn restored_records_start_offline() {
        let mut inventory = Inventory::default();
        let (_, record) = inventory.upsert(identity("a", "1"), LinkKind::Ethernet, Vec::new(), 1);

        let restored = Inventory::from_records(vec![record]);
        let key = DeviceKey::new("helios", "a");
        assert_eq!(restored.get(&key).unwrap().presence, Presence::Offline);
    }

    #[test]
    fn direct_links_rank_ahead_of_recovery() {
        assert!(link_rank(&LinkKind::UsbNetwork) < link_rank(&LinkKind::Ethernet));
        assert!(link_rank(&LinkKind::Ethernet) < link_rank(&LinkKind::UsbBoot));
    }
}
