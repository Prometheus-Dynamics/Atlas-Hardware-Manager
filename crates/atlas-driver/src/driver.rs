use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Candidate, Capabilities, DriverError, Family, HealthCheck, Identity, Link, LinkKind};

/// Static description of a driver, used for matching and ranking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverManifest {
    pub family: Family,
    /// Human-readable driver name, for example `HeliOS`.
    pub name: String,
    pub version: String,
    /// Link kinds this driver can discover devices on.
    pub link_kinds: Vec<LinkKind>,
    /// Higher wins when two drivers claim the same family. Default 0.
    pub priority: i32,
}

impl DriverManifest {
    pub fn handles(&self, link: &LinkKind) -> bool {
        self.link_kinds.iter().any(|kind| kind.same_variant(link))
    }
}

/// One device family's implementation.
#[async_trait]
pub trait Driver: Send + Sync {
    fn manifest(&self) -> &DriverManifest;

    /// Finds candidates on one link. Must return quickly; slow probing
    /// belongs in [`identify`](Self::identify), which runs in parallel.
    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError>;

    /// Asks a candidate who it is.
    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError>;

    /// The capabilities this driver grants the device.
    fn capabilities(&self, device: &Identity) -> Capabilities;

    /// Devices reachable through this one. Only gateways return anything.
    async fn children(&self, _device: &Identity) -> Result<Vec<Candidate>, DriverError> {
        Ok(Vec::new())
    }

    /// Host readiness checks this driver depends on, such as boot files or
    /// USB permissions. Shown in Settings and `atlas doctor`.
    async fn health(&self) -> Vec<HealthCheck> {
        Vec::new()
    }

    /// Runs a fix offered by one of this driver's health checks
    /// ([`HealthCheck::fix_action`]) and returns what changed.
    async fn fix(&self, action: &str) -> Result<String, DriverError> {
        Err(DriverError::Unsupported(format!("no fix named `{action}`")))
    }
}

/// How a link source tells Atlas that something may have changed: a USB
/// device was plugged in, a network device announced itself. Cheap to clone
/// and safe to call from any thread, as often as needed; Atlas coalesces.
#[derive(Clone)]
pub struct ChangeNotifier(Arc<dyn Fn() + Send + Sync>);

impl ChangeNotifier {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(notify))
    }

    pub fn notify(&self) {
        (self.0)();
    }
}

impl fmt::Debug for ChangeNotifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChangeNotifier")
    }
}

/// Supplies the links present on this computer, for example USB network
/// interfaces or USB boot devices. Transports implement this.
#[async_trait]
pub trait LinkSource: Send + Sync {
    async fn links(&self) -> Vec<Link>;

    /// A short name for status lines, for example `USB` or `Network`.
    fn name(&self) -> &str {
        "links"
    }

    /// Starts pushing change notices to `notify`, for example from USB
    /// hotplug or mDNS announcements. Returns false when this source cannot
    /// watch and must be polled; Atlas then rescans it on a slow timer.
    fn watch(&self, _notify: ChangeNotifier) -> bool {
        false
    }

    /// Host readiness checks for this kind of link.
    async fn health(&self) -> Vec<HealthCheck> {
        Vec::new()
    }
}
