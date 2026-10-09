//! Actions the on-device agent claims through Orion: `locate` and `reboot`;
//! when the node's agent handles updates, `update.cancel` (stop a download
//! or stage, or forget a staged update) and `update.rollback` (restart into
//! the previous confirmed version); and, when it claims `clock.set`, setting
//! the board's clock from this computer's.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{ActionsCapability, DeviceAction, DriverError, Identity};
use orion_control_plane::{
    ActionRequest, ActionResult, ActionState, ActionTarget, TypedConfigValue, action_names,
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
    let result = transport.run_action(request).await?;
    wait_for(transport, result, &id, &name, timeout, on_progress).await
}

/// Waits up to `timeout` for the submitted action `id` to reach a final
/// state, starting from what the submission returned.
async fn wait_for(
    transport: &dyn OrionTransport,
    mut result: ActionResult,
    id: &str,
    name: &str,
    timeout: Duration,
    on_progress: &(dyn Fn(u16) + Send + Sync),
) -> Result<ActionResult, DriverError> {
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
            .query_action(id)
            .await?
            .ok_or_else(|| DriverError::Unreachable(format!("Orion no longer tracks {name}")))?;
    }
}

/// The Orion action that sets the board's clock (board-agent's `clock.set`),
/// offered as Atlas's `set-clock` so the SSH form and this one are one control.
pub(crate) const CLOCK_SET: &str = "clock.set";
const SET_CLOCK_ID: &str = "set-clock";

/// The node actions a node's agent claims: from its `action.claimed` list,
/// or, for an agent from before that list, what holding `update` implied.
pub(crate) fn legacy_claims() -> BTreeSet<String> {
    [
        action_names::UPDATE,
        action_names::UPDATE_CANCEL,
        action_names::UPDATE_ROLLBACK,
        action_names::REBOOT,
        action_names::LOCATE,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(crate) struct OrionActions {
    transport: Arc<dyn OrionTransport>,
    node: NodeId,
    /// What the node's agent claims (empty: no agent).
    claimed: BTreeSet<String>,
}

impl OrionActions {
    pub(crate) fn new(
        transport: Arc<dyn OrionTransport>,
        node: NodeId,
        claimed: BTreeSet<String>,
    ) -> Self {
        Self {
            transport,
            node,
            claimed,
        }
    }

    /// The Orion action behind an Atlas action id, if this node offers it.
    fn orion_name(&self, action_id: &str) -> Option<&'static str> {
        match action_id {
            "locate" => Some(action_names::LOCATE),
            "reboot" => Some(action_names::REBOOT),
            action_names::UPDATE_CANCEL | action_names::UPDATE_ROLLBACK
                if self.claimed.contains(action_names::UPDATE) =>
            {
                Some(if action_id == action_names::UPDATE_CANCEL {
                    action_names::UPDATE_CANCEL
                } else {
                    action_names::UPDATE_ROLLBACK
                })
            }
            SET_CLOCK_ID if self.claimed.contains(CLOCK_SET) => Some(CLOCK_SET),
            _ => None,
        }
    }
}

#[async_trait]
impl ActionsCapability for OrionActions {
    fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
        let mut actions = vec![
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
        ];
        if self.claimed.contains(action_names::UPDATE) {
            actions.extend([
                DeviceAction {
                    id: action_names::UPDATE_CANCEL.into(),
                    label: "Cancel update".into(),
                    destructive: false,
                },
                DeviceAction {
                    id: action_names::UPDATE_ROLLBACK.into(),
                    label: "Go back to the previous version".into(),
                    destructive: true,
                },
            ]);
        }
        if self.claimed.contains(CLOCK_SET) {
            actions.push(DeviceAction {
                id: SET_CLOCK_ID.into(),
                label: "Set clock from this computer".into(),
                destructive: false,
            });
        }
        actions
    }

    /// Orion goes first for the actions it offers; Atlas falls back to the
    /// board's own transport when Orion can't be reached.
    fn preferred(&self) -> bool {
        true
    }

    async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
        let Some(name) = self.orion_name(action_id) else {
            return Err(DriverError::Unsupported(format!("no action {action_id}")));
        };
        let mut args = BTreeMap::new();
        if name == CLOCK_SET {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| DriverError::Other(error.to_string()))?;
            args.insert(
                "unix".to_owned(),
                TypedConfigValue::Int(i64::try_from(now.as_secs()).unwrap_or(i64::MAX)),
            );
        }
        let request = request(&self.node, name, args);
        let id = request.action_id.clone();
        // Unreachable here means it was never sent, so the device's own
        // transport may run it instead (Capabilities::fill_from). Once sent,
        // losing track of it is an ordinary error: running it again could
        // restart the board twice.
        let sent = self.transport.run_action(request).await?;
        wait_for(
            self.transport.as_ref(),
            sent,
            &id,
            name,
            Duration::from_secs(30),
            &|_| {},
        )
        .await
        .map_err(|error| match error {
            DriverError::Unreachable(reason) => {
                DriverError::Other(format!("{name} was sent through Orion, then: {reason}"))
            }
            error => error,
        })?;
        Ok(())
    }
}
