//! The Orion nodes Atlas currently knows, keyed by board serial.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use atlas_driver::attributes::{BOARD_SERIAL, normalize_board_serial};
use atlas_driver::{
    ActionsCapability, Capabilities, CapabilitySource, HealthCheck, Identity, Link, LinkSource,
    TelemetryCapability, UpdateCapability,
};
use orion_control_plane::NodeRecord;

use crate::actions::OrionActions;
use crate::metrics::OrionTelemetry;
use crate::transport::{BundleHost, OrionTransport};
use crate::update::OrionUpdate;

/// Orion's view of the fleet. Registered with Atlas twice: as a link source
/// (each scan refreshes it, contributing no links of its own) and as a
/// capability source (it adds Orion's capabilities to matching devices).
pub struct OrionDirectory {
    pub(crate) transport: Arc<dyn OrionTransport>,
    bundles: Option<Arc<dyn BundleHost>>,
    nodes: RwLock<HashMap<String, NodeRecord>>,
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
            last_error: Mutex::new(None),
        })
    }

    /// Re-reads the node list. Nodes without a usable board serial can't be
    /// matched to a device and are skipped.
    pub async fn refresh(&self) -> Result<usize, atlas_driver::DriverError> {
        let result = self.transport.nodes().await;
        let mut error = self
            .last_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            Ok(records) => {
                let map: HashMap<String, NodeRecord> = records
                    .into_iter()
                    .filter_map(|record| {
                        let serial = record.host.as_ref()?.board_serial.as_deref()?;
                        Some((normalize_board_serial(serial)?, record))
                    })
                    .collect();
                let count = map.len();
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
        vec![match error {
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
        }]
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
        Capabilities {
            telemetry: Some(Arc::new(OrionTelemetry::new(
                self.transport.clone(),
                id.clone(),
                &node,
            )) as Arc<dyn TelemetryCapability>),
            actions: Some(
                Arc::new(OrionActions::new(self.transport.clone(), id.clone()))
                    as Arc<dyn ActionsCapability>,
            ),
            update: self.bundles.clone().and_then(|bundles| {
                device.attributes.get(BOARD_SERIAL).map(|serial| {
                    Arc::new(OrionUpdate::new(
                        self.transport.clone(),
                        bundles,
                        id,
                        serial.clone(),
                    )) as Arc<dyn UpdateCapability>
                })
            }),
            ..Capabilities::default()
        }
    }
}
