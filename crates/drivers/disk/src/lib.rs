//! The removable-disk driver. It finds the cards and removable disks that an
//! image can be written to, and says what each one is. It grants `Flash` in
//! the `RemovableDisk` state and nothing else. The write itself is the
//! helper's job (`atlas-helper`), not this driver's.
//!
//! The manifest declares no link kinds yet. The registry routes a link only
//! to drivers that handle its kind, and no kind names local disks: adding one
//! is a change to `crates/core/types` and waits for an A-series PR. Until
//! then the registry never calls this driver; its functions are tested
//! directly.
//!
//! This crate depends on no other driver. It is named `atlas-driver-disk`
//! because `atlas-disk` is the flash crate in `crates/flash/disk`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_disk::{Disk, DiskSource, SysfsDisks, check_target};
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
};

/// The family every removable disk belongs to: the disk itself, not a board.
pub const FAMILY: &str = "removable-disk";

pub struct DiskDriver {
    manifest: DriverManifest,
    source: Arc<dyn DiskSource>,
}

impl DiskDriver {
    /// A driver that reads disks from `source`. Tests pass a made-up list.
    pub fn new(source: Arc<dyn DiskSource>) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Removable disk".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: Vec::new(),
                priority: 0,
            },
            source,
        }
    }

    /// A driver that reads the disks the kernel shows.
    pub fn sysfs() -> Self {
        Self::new(Arc::new(SysfsDisks))
    }
}

impl fmt::Debug for DiskDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskDriver")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

/// Whether a disk may be written. The same rules the helper applies.
fn writable(disk: &Disk) -> bool {
    check_target(disk, None).is_ok()
}

/// The stable key: the serial when the disk has one, else its USB port, else its name.
fn key_for(disk: &Disk) -> DeviceKey {
    let serial = disk
        .serial
        .clone()
        .or_else(|| disk.usb_port.as_ref().map(|port| format!("usb-{port}")))
        .unwrap_or_else(|| disk.name.clone());
    DeviceKey::new(FAMILY, serial)
}

fn attributes_for(disk: &Disk) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::from([
        ("size-bytes".to_string(), disk.size_bytes.to_string()),
        ("device".to_string(), disk.path.display().to_string()),
    ]);
    if let Some(port) = &disk.usb_port {
        attributes.insert("usb-port".into(), port.clone());
    }
    attributes
}

#[async_trait]
impl Driver for DiskDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        let disks = self
            .source
            .disks()
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        Ok(disks
            .iter()
            .filter(|disk| writable(disk))
            .map(|disk| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: disk.path.display().to_string(),
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let disks = self
            .source
            .disks()
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        let disk = disks
            .iter()
            .find(|disk| disk.path.display().to_string() == candidate.address && writable(disk))
            .ok_or_else(|| {
                DriverError::Unreachable(format!(
                    "no writable disk at {}; it may have been unplugged",
                    candidate.address
                ))
            })?;
        Ok(Identity {
            key: key_for(disk),
            model: disk.description(),
            state: DeviceState::RemovableDisk,
            versions: BTreeMap::new(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: attributes_for(disk),
        })
    }

    fn capabilities(&self, _device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds([CapabilityKind::Flash])
    }
}
