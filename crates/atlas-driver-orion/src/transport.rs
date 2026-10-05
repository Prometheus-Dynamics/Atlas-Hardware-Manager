//! What Atlas needs from Orion, independent of how it gets there.

use async_trait::async_trait;
use atlas_driver::{Artifact, DriverError};
use orion_control_plane::{ActionRequest, ActionResult, NodeRecord, StatusEntry, StatusQuery};

/// One connection to an Orion cluster, as an operator. Every call may fail
/// with `DriverError::Unreachable` when Orion can't be reached.
#[async_trait]
pub trait OrionTransport: Send + Sync {
    /// Every node Orion knows, with its observed host facts.
    async fn nodes(&self) -> Result<Vec<NodeRecord>, DriverError>;

    /// Status-lane entries (volatile metrics and durable `update.*` keys).
    async fn status(&self, query: StatusQuery) -> Result<Vec<StatusEntry>, DriverError>;

    /// Submits an action. Resubmitting the same `action_id` is idempotent.
    async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError>;

    /// The newest result for one action, if Orion still tracks it.
    async fn query_action(&self, action_id: &str) -> Result<Option<ActionResult>, DriverError>;
}

/// Makes a release file reachable by URL for devices to pull. Update bytes
/// never travel over Orion; the device downloads the bundle from Atlas.
pub trait BundleHost: Send + Sync {
    fn url_for(&self, artifact: &Artifact) -> Result<String, DriverError>;
}
