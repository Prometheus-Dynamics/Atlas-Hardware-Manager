//! The real transport: Orion's remote operator client (signed orion+tcp,
//! `orion_client::remote`). Atlas connects as an operator, never a cluster
//! member. The node it connects to forwards actions and status queries to
//! the node that owns them.
//!
//! The connection is made on first use and dropped on a transport error,
//! so a node that restarts is picked up again on the next call. The node's
//! key is trusted on first use and then pinned (`pin_file`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::DriverError;
use orion_client::remote::{NodeTrust, OperatorIdentity, RemoteError, RemoteOperator};
use orion_control_plane::{
    ActionRequest, ActionResult, NodeRecord, ResourceRecord, StateSnapshot, StatusEntry,
    StatusQuery,
};
use orion_core::NodeId;
use serde::Serialize;

use crate::clock::ClockServer;
use crate::pairing::{
    OtherKey, PairingLog, enroll_command, is_other_key, is_removed, removed_message,
};
use crate::transport::OrionTransport;

/// What the UI shows about the Orion connection.
#[derive(Clone, Debug, Default, Serialize)]
pub struct OrionConnection {
    pub url: Option<String>,
    /// `operator:<name>`: what an administrator enrolls.
    pub operator_id: String,
    /// `sha256:<32 hex>`, compared with what `orionctl operators list` shows.
    pub fingerprint: String,
    /// The command an administrator runs on a node to let Atlas in.
    pub enroll_command: String,
    pub connected: bool,
    pub enrolled: bool,
    pub node_id: Option<String>,
    pub node_fingerprint: Option<String>,
    pub error: Option<String>,
    /// Set while the node refuses this computer's key because it has another
    /// key enrolled for the operator name: the fix to show.
    pub other_key: Option<OtherKey>,
}

/// A live session, and what serves the clock over it: dropping the link
/// stops the offers with the connection.
struct Link {
    operator: RemoteOperator,
    _clock: Option<ClockServer>,
}

pub struct RemoteTransport {
    identity: OperatorIdentity,
    pin_file: Option<PathBuf>,
    url: Mutex<Option<String>>,
    session: tokio::sync::Mutex<Option<Link>>,
    /// Whether the live session's node key was pinned before it connected
    /// (see [`OrionTransport::authenticated`]).
    trusted: AtomicBool,
    state: Mutex<OrionConnection>,
    pairing_log: Option<Arc<PairingLog>>,
}

/// The actions Atlas runs through Orion; the enroll command grants these.
pub(crate) const ACTIONS: [&str; 32] = [
    "update",
    "update.cancel",
    "update.rollback",
    "reboot",
    "locate",
    "clock.set",
    // The board's devices (Lemnos): set, restore and release controls,
    // switch a power switch, and calibrate the IMU and magnetometer.
    "set",
    "restore",
    "release",
    "power.set",
    "calibration.start",
    "calibration.stop",
    "calibration.apply",
    "calibration.discard",
    "calibration.reset",
    "calibration.status",
    // The board's USB route onto the robot network (board-agent).
    "usb-route.set",
    // A slots board's slots (board-agent).
    "slot.boot-once",
    "slot.set-default",
    "slot.remove",
    "slot.pin",
    // A power switch's reset, and the status ring's looks and brightness.
    "power.reset",
    "looks.preset.list",
    "looks.preset.show",
    "looks.preset.apply",
    "looks.preset.save",
    "looks.preset.delete",
    "looks.show_inline",
    "looks.look",
    "looks.off",
    "looks.locate",
    "light.brightness",
];

impl RemoteTransport {
    pub fn new(identity: OperatorIdentity, url: Option<String>, pin_file: Option<PathBuf>) -> Self {
        let operator_id = identity.operator_id().to_string();
        let fingerprint = identity.fingerprint();
        let enroll_command = enroll_command(&operator_id, &fingerprint);
        Self {
            identity,
            pin_file,
            state: Mutex::new(OrionConnection {
                url: url.clone(),
                operator_id,
                fingerprint,
                enroll_command,
                ..OrionConnection::default()
            }),
            url: Mutex::new(url),
            session: tokio::sync::Mutex::new(None),
            trusted: AtomicBool::new(false),
            pairing_log: None,
        }
    }

    /// Ends the live session (and its clock offers), so the next call
    /// connects again.
    async fn drop_session(&self) {
        *self.session.lock().await = None;
        self.trusted.store(false, Ordering::SeqCst);
    }

    /// Logs a board's refusal of this computer's key (once per board and key).
    pub fn with_pairing_log(mut self, log: Arc<PairingLog>) -> Self {
        self.pairing_log = Some(log);
        self
    }

    /// Points Atlas at another node (or none); the next call reconnects.
    pub async fn set_url(&self, url: Option<String>) {
        *self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = url.clone();
        self.drop_session().await;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.url = url;
        state.connected = false;
        state.enrolled = false;
        state.node_id = None;
        state.node_fingerprint = None;
        state.error = None;
        state.other_key = None;
        // A new node has its own key: trust it on first use again.
        if let Some(pin) = &self.pin_file {
            let _ = std::fs::remove_file(pin);
        }
    }

    pub fn connection(&self) -> OrionConnection {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Shared-key enrollment, for nodes built with an enrollment key.
    pub async fn enroll_with_key(&self, key: &str) -> Result<(), DriverError> {
        let session = self.session().await?;
        let result = session.enroll_with_key(key.as_bytes()).await;
        self.record(&result);
        if result.is_ok() {
            // The next call introduces Atlas again, so the welcome carries
            // what the enrollment granted (the clock's grant, too).
            self.drop_session().await;
        }
        result.map(|_| ()).map_err(|error| self.error(&error))
    }

    fn trust(&self) -> NodeTrust {
        self.pin_file
            .as_ref()
            .and_then(|pin| std::fs::read(pin).ok())
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
            .map_or(NodeTrust::FirstUse, NodeTrust::Key)
    }

    async fn session(&self) -> Result<RemoteOperator, DriverError> {
        let mut guard = self.session.lock().await;
        if let Some(link) = guard.as_ref() {
            return Ok(link.operator.clone());
        }
        let url = self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| DriverError::Unsupported("Orion isn't set up in Atlas".into()))?;
        let trust = self.trust();
        // Only a key pinned before this connection proves the node: a first
        // use takes whatever key the node shows.
        let pinned = matches!(trust, NodeTrust::Key(_));
        let connected = RemoteOperator::connect(&url, self.identity.clone(), trust).await;
        let session = match connected {
            Ok(session) => session,
            Err(error) => {
                let other = is_other_key(&error).then(|| self.refused(&url));
                let failed = match &other {
                    Some(key) => DriverError::Unsupported(key.message()),
                    None if is_removed(&error) => {
                        DriverError::Unsupported(removed_message(&self.identity))
                    }
                    None => crate::protocol::unreachable(&url, &error),
                };
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.connected = false;
                state.error = Some(match (&other, &failed) {
                    (Some(key), _) => key.message(),
                    (None, DriverError::Incompatible(message)) => message.clone(),
                    (None, DriverError::Unsupported(message)) if is_removed(&error) => {
                        message.clone()
                    }
                    _ => error.to_string(),
                });
                state.other_key = other;
                return Err(failed);
            }
        };
        if let Some(pin) = &self.pin_file
            && !pin.exists()
        {
            let _ = std::fs::write(pin, session.node_public_key());
        }
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.connected = true;
            state.enrolled = session.is_enrolled();
            state.node_id = Some(session.node_id().to_string());
            state.node_fingerprint = Some(session.node_fingerprint());
            state.error = None;
        }
        // The clock is offered on this connection, for as long as it lives.
        let clock = ClockServer::start(&session);
        self.trusted.store(pinned, Ordering::SeqCst);
        *guard = Some(Link {
            operator: session.clone(),
            _clock: clock,
        });
        Ok(session)
    }

    fn record<T>(&self, result: &Result<T, RemoteError>) {
        let other = match result {
            Err(error) if is_other_key(error) => Some(self.refused(&self.board())),
            _ => None,
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            Ok(_) => {
                state.enrolled = true;
                state.error = None;
                state.other_key = None;
            }
            Err(error) if is_removed(error) => {
                state.enrolled = false;
                state.error = Some(removed_message(&self.identity));
                state.other_key = None;
            }
            Err(error) if error.is_not_enrolled() => {
                state.enrolled = false;
                state.error = None;
                state.other_key = None;
            }
            Err(_) if other.is_some() => {
                state.enrolled = false;
                state.error = other.as_ref().map(OtherKey::message);
                state.other_key = other;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    /// The node at `board` refused this computer's key: logged once.
    fn refused(&self, board: &str) -> OtherKey {
        let key = OtherKey::for_identity(&self.identity);
        if let Some(log) = &self.pairing_log {
            log.refused(board, &key);
        }
        key
    }

    fn board(&self) -> String {
        self.url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_default()
    }

    fn error(&self, error: &RemoteError) -> DriverError {
        if is_removed(error) {
            return DriverError::Unsupported(removed_message(&self.identity));
        }
        if error.is_not_enrolled() {
            let command = self.connection().enroll_command;
            return DriverError::Unsupported(format!(
                "This computer isn't enrolled with Orion yet; on the device run: {command}"
            ));
        }
        if is_other_key(error) {
            return DriverError::Unsupported(OtherKey::for_identity(&self.identity).message());
        }
        crate::protocol::mismatch(error)
            .unwrap_or_else(|| DriverError::Unreachable(format!("Orion: {error}")))
    }

    /// Runs one call, dropping the session after a failure so the next call
    /// reconnects (the node may have restarted).
    async fn call<T, F, Fut>(&self, work: F) -> Result<T, DriverError>
    where
        F: FnOnce(RemoteOperator) -> Fut,
        Fut: std::future::Future<Output = Result<T, RemoteError>>,
    {
        let session = self.session().await?;
        let result = work(session).await;
        self.record(&result);
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                if !error.is_not_enrolled() {
                    self.drop_session().await;
                    self.state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .connected = false;
                }
                Err(self.error(&error))
            }
        }
    }
}

/// Every resource in the snapshot, merged as `nodes()` merges nodes: the
/// desired records, with the observed ones over them. Each comes with the node
/// its provider is registered on (providers are only in desired state).
fn effective_resources(snapshot: StateSnapshot) -> Vec<(ResourceRecord, Option<NodeId>)> {
    let state = snapshot.state;
    let mut resources = state.desired.resources;
    resources.extend(state.observed.resources);
    let providers = state.desired.providers;
    resources
        .into_values()
        .map(|resource| {
            let node = providers
                .get(&resource.provider_id)
                .map(|provider| provider.node_id.clone());
            (resource, node)
        })
        .collect()
}

#[async_trait]
impl OrionTransport for RemoteTransport {
    fn configured(&self) -> bool {
        self.url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    fn authenticated(&self) -> bool {
        self.trusted.load(Ordering::SeqCst)
    }

    async fn nodes(&self) -> Result<Vec<NodeRecord>, DriverError> {
        self.call(|session| async move { session.nodes().await })
            .await
    }

    async fn resources(&self) -> Result<Vec<(ResourceRecord, Option<NodeId>)>, DriverError> {
        self.call(|session| async move { session.state_snapshot().await })
            .await
            .map(effective_resources)
    }

    async fn status(&self, query: StatusQuery) -> Result<Vec<StatusEntry>, DriverError> {
        self.call(|session| async move { session.status(query).await })
            .await
    }

    async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError> {
        self.call(|session| async move { session.run_action(request).await })
            .await
    }

    async fn query_action(&self, action_id: &str) -> Result<Option<ActionResult>, DriverError> {
        let id = action_id.to_string();
        self.call(|session| async move { session.query_action(&id).await })
            .await
    }

    async fn call_action(
        &self,
        request: ActionRequest,
        timeout: Duration,
    ) -> Result<ActionResult, DriverError> {
        let id = request.action_id.clone();
        let waited = self
            .call(|session| async move {
                match session.call_action(request, timeout).await {
                    // Still running: the caller decides, as with the default.
                    Err(RemoteError::ActionTimeout { .. }) => Ok(None),
                    other => other.map(Some),
                }
            })
            .await?;
        match waited {
            Some(result) => Ok(result),
            None => self
                .query_action(&id)
                .await?
                .ok_or_else(|| DriverError::Other(format!("Orion no longer tracks action {id}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use orion_control_plane::{
        ClusterStateEnvelope, DesiredClusterState, ObservedClusterState, ProviderRecord,
        ResourceRecord, StateSnapshot,
    };
    use orion_core::{ProviderId, ResourceId, ResourceType};

    use super::*;

    fn resource(id: &str, provider: &str) -> ResourceRecord {
        ResourceRecord::builder(
            ResourceId::new(id),
            ResourceType::new("lemnos.device"),
            ProviderId::new(provider),
        )
        .build()
    }

    #[test]
    fn each_resource_comes_with_its_providers_node() {
        let mut desired = DesiredClusterState::default();
        desired.resources.insert(
            ResourceId::new("lemnos.raze.imu"),
            resource("lemnos.raze.imu", "lemnos"),
        );
        desired.resources.insert(
            ResourceId::new("lemnos.other.gps"),
            resource("lemnos.other.gps", "lemnos-other"),
        );
        desired.resources.insert(
            ResourceId::new("lemnos.raze.orphan"),
            resource("lemnos.raze.orphan", "gone"),
        );
        desired.providers.insert(
            ProviderId::new("lemnos"),
            ProviderRecord::builder(ProviderId::new("lemnos"), NodeId::new("raze-node")).build(),
        );
        desired.providers.insert(
            ProviderId::new("lemnos-other"),
            ProviderRecord::builder(ProviderId::new("lemnos-other"), NodeId::new("other-node"))
                .build(),
        );
        let snapshot = StateSnapshot {
            state: ClusterStateEnvelope::new(
                desired,
                ObservedClusterState::default(),
                Default::default(),
            ),
        };

        let nodes: Vec<(String, Option<String>)> = effective_resources(snapshot)
            .into_iter()
            .map(|(record, node)| {
                (
                    record.resource_id.as_str().to_string(),
                    node.map(|node| node.as_str().to_string()),
                )
            })
            .collect();
        assert_eq!(
            nodes,
            vec![
                ("lemnos.other.gps".into(), Some("other-node".into())),
                ("lemnos.raze.imu".into(), Some("raze-node".into())),
                ("lemnos.raze.orphan".into(), None),
            ]
        );
    }
}
