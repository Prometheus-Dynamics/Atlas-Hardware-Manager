//! Actions the on-device agent claims through Orion: `locate` and `reboot`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{ActionsCapability, DeviceAction, DriverError, Identity};
use orion_control_plane::{
    ActionRequest, ActionResult, ActionState, ActionTarget, TypedConfigValue,
};
use orion_core::NodeId;

use crate::transport::OrionTransport;

const POLL: Duration = Duration::from_millis(500);

/// A unique, readable action id: `atlas-<name>-<millis>-<n>`.
pub(crate) fn action_id(name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    format!(
        "atlas-{name}-{millis}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn request(
    node: &NodeId,
    name: &str,
    args: BTreeMap<String, TypedConfigValue>,
) -> ActionRequest {
    ActionRequest {
        action_id: action_id(name),
        target: ActionTarget::Node(node.clone()),
        name: name.into(),
        args,
        deadline_ms: 0,
        // The node stamps the authenticated requester.
        requested_by: String::new(),
    }
}

/// Submits `request` and waits up to `timeout` for a final state, calling
/// `on_progress` with per-mille progress while it runs.
pub(crate) async fn run_and_wait(
    transport: &dyn OrionTransport,
    request: ActionRequest,
    timeout: Duration,
    on_progress: &(dyn Fn(u16) + Send + Sync),
) -> Result<ActionResult, DriverError> {
    let id = request.action_id.clone();
    let name = request.name.clone();
    let mut result = transport.run_action(request).await?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match &result.state {
            ActionState::Succeeded => return Ok(result),
            ActionState::Failed { reason } | ActionState::Rejected { reason } => {
                return Err(DriverError::Other(format!("{name}: {reason}")));
            }
            ActionState::TimedOut => {
                return Err(DriverError::Other(format!(
                    "{name} timed out on the device"
                )));
            }
            ActionState::Running {
                progress: Some(progress),
            } => on_progress(*progress),
            ActionState::Accepted | ActionState::Running { progress: None } => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(DriverError::Unreachable(format!(
                "{name} gave no result in time"
            )));
        }
        tokio::time::sleep(POLL).await;
        result = transport
            .query_action(&id)
            .await?
            .ok_or_else(|| DriverError::Unreachable(format!("Orion no longer tracks {name}")))?;
    }
}

pub(crate) struct OrionActions {
    transport: Arc<dyn OrionTransport>,
    node: NodeId,
}

impl OrionActions {
    pub(crate) fn new(transport: Arc<dyn OrionTransport>, node: NodeId) -> Self {
        Self { transport, node }
    }
}

#[async_trait]
impl ActionsCapability for OrionActions {
    fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
        vec![
            DeviceAction {
                id: "locate".into(),
                label: "Find it".into(),
                destructive: false,
            },
            DeviceAction {
                id: "reboot".into(),
                label: "Restart".into(),
                destructive: false,
            },
        ]
    }

    async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
        if !matches!(action_id, "locate" | "reboot") {
            return Err(DriverError::Unsupported(format!("no action {action_id}")));
        }
        let request = request(&self.node, action_id, BTreeMap::new());
        run_and_wait(
            self.transport.as_ref(),
            request,
            Duration::from_secs(30),
            &|_| {},
        )
        .await?;
        Ok(())
    }
}
