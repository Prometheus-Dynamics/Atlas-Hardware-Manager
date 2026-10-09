//! The Orion nodes Atlas currently knows, keyed by board serial.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::attributes::{BOARD_SERIAL, normalize_board_serial};
use atlas_driver::{
    ActionsCapability, Capabilities, CapabilitySource, HealthCheck, Identity, Link, LinkSource,
    StatusCapability, TelemetryCapability, UpdateCapability,
};
use orion_control_plane::{
    NodeRecord, StatusQuery, StatusSubject, TypedConfigValue, update_action,
};

/// The status key board-agent lists its claimed actions under.
const CLAIMED_KEY: &str = "action.claimed";

use crate::actions::{OrionActions, legacy_claims};
use crate::metrics::OrionTelemetry;
use crate::status::OrionStatus;
use crate::transport::{BundleHost, OrionTransport};
use crate::update::OrionUpdate;

/// Orion's view of the fleet. Registered with Atlas twice: as a link source
/// (each scan refreshes it, contributing no links of its own) and as a
/// capability source (it adds Orion's capabilities to matching devices).
pub struct OrionDirectory {
    pub(crate) transport: Arc<dyn OrionTransport>,
    bundles: Option<Arc<dyn BundleHost>>,
    nodes: RwLock<HashMap<String, NodeRecord>>,
    /// What each node's device agent claims, by node id: its `action.claimed`
    /// list, or for an older agent that publishes only `update.state`, the
    /// actions that implied. Nodes without an agent are absent.
    agents: RwLock<HashMap<String, BTreeSet<String>>>,
    /// How often an update polls the board's state, in milliseconds.
    update_poll_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl OrionDirectory {
    /// `bundles` enables updates; without it Orion adds readings and actions.
    pub fn new(
        transport: Arc<dyn OrionTransport>,
        bundles: Option<Arc<dyn BundleHost>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            transport,
            bundles,
            nodes: RwLock::new(HashMap::new()),
            agents: RwLock::new(HashMap::new()),
            update_poll_ms: AtomicU64::new(3_000),
            last_error: Mutex::new(None),
        })
    }

    /// Re-reads the node list. Nodes without a usable board serial can't be
    /// matched to a device and are skipped.
    pub async fn refresh(&self) -> Result<usize, atlas_driver::DriverError> {
        let result = match self.transport.nodes().await {
            Ok(records) => {
                let map: HashMap<String, NodeRecord> = records
                    .into_iter()
                    .filter_map(|record| {
                        let serial = record.host.as_ref()?.board_serial.as_deref()?;
                        Some((normalize_board_serial(serial)?, record))
                    })
                    .collect();
                let agents = self.agents_among(map.values()).await;
                Ok((map, agents))
            }
            Err(failure) => Err(failure),
        };
        let mut error = self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            Ok((map, agents)) => {
                let count = map.len();
                *self
                    .agents
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = agents;
                *self
                    .nodes
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = map;
                *error = None;
                Ok(count)
            }
            Err(failure) => {
                *error = Some(failure.to_string());
                Err(failure)
            }
        }
    }

    /// How often updates poll the board's `update.*` keys and boot id (3 s
    /// by default; tests make it shorter).
    pub fn set_update_poll(&self, every: Duration) {
        let millis = u64::try_from(every.as_millis()).unwrap_or(u64::MAX).max(1);
        self.update_poll_ms.store(millis, Ordering::Relaxed);
    }

    /// What the device agent of each node among `records` claims (two
    /// status queries per node, all nodes at once).
    async fn agents_among<'a>(
        &self,
        records: impl Iterator<Item = &'a NodeRecord>,
    ) -> HashMap<String, BTreeSet<String>> {
        let queries = records.map(|record| async move {
            let key = |prefix: &str| {
                let prefix = prefix.to_owned();
                async move {
                    self.transport
                        .status(StatusQuery {
                            subject: Some(StatusSubject::Node(record.node_id.clone())),
                            key_prefix: Some(prefix.clone()),
                        })
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .find(|entry| entry.key == prefix)
                }
            };
            let claimed = match key(CLAIMED_KEY).await.map(|entry| entry.value) {
                Some(TypedConfigValue::String(list)) => Some(
                    list.split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .collect(),
                ),
                _ => key(update_action::KEY_STATE).await.map(|_| legacy_claims()),
            };
            claimed.map(|claimed| (record.node_id.to_string(), claimed))
        });
        futures::future::join_all(queries)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// The Orion node running on this device's board, if any.
    pub fn node_for(&self, device: &Identity) -> Option<NodeRecord> {
        let serial = device.attributes.get(BOARD_SERIAL)?;
        self.node_by_serial(serial)
    }

    pub(crate) fn node_by_serial(&self, board_serial: &str) -> Option<NodeRecord> {
        self.nodes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&board_serial.to_ascii_lowercase())
            .cloned()
    }
}

#[async_trait]
impl LinkSource for OrionDirectory {
    async fn links(&self) -> Vec<Link> {
        // Orion contributes capabilities, not links: refresh before each scan.
        if self.transport.configured() {
            let _ = self.refresh().await;
        } else {
            self.nodes
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
        }
        Vec::new()
    }

    fn name(&self) -> &str {
        "Orion"
    }

    async fn health(&self) -> Vec<HealthCheck> {
        if !self.transport.configured() {
            return Vec::new();
        }
        let error = self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let count = self
            .nodes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let mut checks = vec![match error {
            None => HealthCheck::ok(
                "orion",
                "Orion",
                format!("Connected; {count} nodes with a board serial."),
            ),
            Some(error) => HealthCheck::warning(
                "orion",
                "Orion",
                format!("Orion can't be reached: {error}"),
                "Devices still work over USB, the network identity, and SSH; Orion adds readings, actions, and updates when it's back.",
            ),
        }];
        checks.extend(self.bundles.as_ref().and_then(|bundles| bundles.health()));
        checks
    }
}

impl CapabilitySource for OrionDirectory {
    fn capabilities_for(&self, device: &Identity) -> Capabilities {
        let Some(node) = device
            .attributes
            .get(BOARD_SERIAL)
            .and_then(|serial| self.node_by_serial(serial))
        else {
            return Capabilities::default();
        };
        let id = node.node_id.clone();
        let claimed = self
            .agents
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id.as_str())
            .cloned()
            .unwrap_or_default();
        Capabilities {
            telemetry: Some(Arc::new(OrionTelemetry::new(
                self.transport.clone(),
                id.clone(),
                &node,
            )) as Arc<dyn TelemetryCapability>),
            actions: Some(Arc::new(OrionActions::new(
                self.transport.clone(),
                id.clone(),
                claimed,
            )) as Arc<dyn ActionsCapability>),
            status: Some(Arc::new(OrionStatus {
                transport: self.transport.clone(),
                node: id.clone(),
            }) as Arc<dyn StatusCapability>),
            update: self.bundles.clone().and_then(|bundles| {
                device.attributes.get(BOARD_SERIAL).map(|serial| {
                    Arc::new(OrionUpdate::new(
                        self.transport.clone(),
                        bundles,
                        id,
                        serial.clone(),
                        Duration::from_millis(self.update_poll_ms.load(Ordering::Relaxed)),
                    )) as Arc<dyn UpdateCapability>
                })
            }),
            ..Capabilities::default()
        }
    }
}
