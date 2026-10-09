//! What Atlas needs from Orion, independent of how it gets there.

use async_trait::async_trait;
use std::net::IpAddr;
use std::time::Duration;

use atlas_driver::{Artifact, DriverError, HealthCheck};
use orion_control_plane::{
    ActionRequest, ActionResult, NodeRecord, ResourceRecord, StatusEntry, StatusQuery,
};
use orion_core::NodeId;

/// One connection to an Orion cluster, as an operator. Every call may fail
/// with `DriverError::Unreachable` when Orion can't be reached.
#[async_trait]
pub trait OrionTransport: Send + Sync {
    /// False while Atlas has no Orion to talk to; the directory then stays
    /// quiet instead of reporting a connection problem.
    fn configured(&self) -> bool {
        true
    }

    /// Every node Orion knows, with its observed host facts. A rebooted
    /// node's new `boot_id` shows up here (not on the status lane) once it
    /// is back and has synced; until then the old record may still appear.
    async fn nodes(&self) -> Result<Vec<NodeRecord>, DriverError>;

    /// Every resource Orion knows, each with the node its provider is
    /// registered on (`None` when that provider isn't known). Resources are
    /// cluster state, so any node can answer. Empty by default.
    async fn resources(&self) -> Result<Vec<(ResourceRecord, Option<NodeId>)>, DriverError> {
        Ok(Vec::new())
    }

    /// Status-lane entries (volatile metrics and durable `update.*` keys).
    /// The status lane is per node and not replicated, so an implementation
    /// must answer for the subject's own node: Orion's operator client
    /// forwards the query there; anything else must connect to that node.
    async fn status(&self, query: StatusQuery) -> Result<Vec<StatusEntry>, DriverError>;

    /// Submits an action. Resubmitting the same `action_id` is idempotent.
    async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError>;

    /// The newest result for one action, if Orion still tracks it.
    async fn query_action(&self, action_id: &str) -> Result<Option<ActionResult>, DriverError>;

    /// Runs `request` and waits up to `timeout` for its final result; the
    /// latest running one when `timeout` passes first. Orion's operator client
    /// waits on the node (request/response actions); this default, for
    /// transports without that, resubmits and then asks every 250 ms.
    async fn call_action(
        &self,
        request: ActionRequest,
        timeout: Duration,
    ) -> Result<ActionResult, DriverError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let id = request.action_id.clone();
        let mut result = self.run_action(request).await?;
        while !result.state.is_terminal() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(250)).await;
            match self.query_action(&id).await? {
                Some(newer) => result = newer,
                None => break,
            }
        }
        Ok(result)
    }
}

/// Makes a release file reachable by URL for devices to pull. Update bytes
/// never travel over Orion; the device downloads the image from Atlas.
pub trait BundleHost: Send + Sync {
    /// A URL the device at `peer` (when its address is known) can fetch
    /// `artifact` from: the host must be an address of this computer that
    /// routes to the device.
    fn url_for(&self, artifact: &Artifact, peer: Option<IpAddr>) -> Result<String, DriverError>;

    /// Whether devices can reach the host, for the health screen.
    fn health(&self) -> Option<HealthCheck> {
        None
    }
}
