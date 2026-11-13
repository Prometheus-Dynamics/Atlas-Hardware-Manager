use async_trait::async_trait;
use atlas_types::{Candidate, CapabilitySet, Family, Identity, Link, LinkKind};
use serde::{Deserialize, Serialize};

use crate::{DriverError, HealthCheck};

/// Static description of a driver, used for matching and ranking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverManifest {
    pub family: Family,
    /// Human-readable driver name.
    pub name: String,
    pub version: String,
    /// Link kinds this driver can discover devices on.
    pub link_kinds: Vec<LinkKind>,
    /// Higher wins when two drivers claim the same family. Default 0.
    pub priority: i32,
}

impl DriverManifest {
    /// Whether this driver can discover devices on `link`.
    pub fn handles(&self, link: &LinkKind) -> bool {
        self.link_kinds.iter().any(|kind| kind.same_kind(link))
    }
}

/// One device family's implementation.
#[async_trait]
pub trait Driver: Send + Sync {
    fn manifest(&self) -> &DriverManifest;

    /// Finds candidates on one link. Must return quickly; slow probing
    /// belongs in [`identify`](Self::identify).
    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError>;

    /// Asks a candidate who it is.
    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError>;

    /// The capabilities this driver grants the device. Synchronous: it reads
    /// what [`identify`](Self::identify) already learned.
    fn capabilities(&self, device: &Identity) -> CapabilitySet;

    /// Host readiness checks this driver depends on, such as boot files or
    /// USB permissions.
    async fn health(&self) -> Vec<HealthCheck> {
        Vec::new()
    }

    /// Runs a fix offered by one of this driver's health checks and returns
    /// what changed.
    async fn fix(&self, action: &str) -> Result<String, DriverError> {
        Err(DriverError::Unsupported(format!("no fix named `{action}`")))
    }
}
