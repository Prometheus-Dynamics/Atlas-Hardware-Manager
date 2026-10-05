//! What Atlas needs from Orion, independent of how it gets there.

use async_trait::async_trait;
use atlas_driver::{Artifact, DriverError};
use orion_control_plane::{ActionRequest, ActionResult, NodeRecord, StatusEntry, StatusQuery};

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

    /// Status-lane entries (volatile metrics and durable `update.*` keys).
    /// The status lane is per node and not replicated, so an implementation
    /// must answer for the subject's own node: Orion's operator client
    /// forwards the query there; anything else must connect to that node.
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
