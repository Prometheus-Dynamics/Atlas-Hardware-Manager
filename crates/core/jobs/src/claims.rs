//! Device claims: which job is working on which device. Two jobs never work on
//! the same device at once; a second one is refused at submit, naming the job
//! that holds it. Read-only work (scan, identify) takes no claim, except that a
//! quiet claim stops the scan from calling a driver about its device.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use atlas_types::DeviceKey;
use serde::{Deserialize, Serialize};

use crate::JobId;

/// The devices a job writes to, and how the scan may treat them meanwhile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceClaim {
    pub devices: Vec<DeviceKey>,
    /// The scan makes no identify calls about this device while the job runs.
    /// For boot-ROM loaders that a probe would disturb.
    #[serde(default)]
    pub quiet: bool,
    /// Where the device sits on its link. A quiet claim matches a candidate by
    /// this address, before any identify has told the scan who it is.
    #[serde(default)]
    pub address: Option<String>,
}

impl DeviceClaim {
    /// A job that writes to no device.
    pub fn none() -> Self {
        Self::default()
    }

    /// A job that writes to these devices.
    pub fn writes(devices: impl IntoIterator<Item = DeviceKey>) -> Self {
        Self {
            devices: devices.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Quiet: the scan leaves the device alone while the job holds it.
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// The device's address on its link, for matching a quiet claim.
    pub fn at(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }
}

#[derive(Clone, Debug)]
struct Hold {
    kind: String,
    claim: DeviceClaim,
}

#[derive(Debug, Default)]
struct Table {
    by_job: BTreeMap<JobId, Hold>,
    by_device: BTreeMap<DeviceKey, JobId>,
}

/// The table of held devices. Shared by the jobs (which take and release
/// holds) and the scan (which reads quiet holds). Clones share one table.
#[derive(Clone, Debug, Default)]
pub struct DeviceHolds {
    table: Arc<Mutex<Table>>,
}

fn lock(table: &Mutex<Table>) -> MutexGuard<'_, Table> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

impl DeviceHolds {
    pub fn new() -> Self {
        Self::default()
    }

    /// The job holding this device, if any.
    pub fn held_by(&self, device: &DeviceKey) -> Option<JobId> {
        lock(&self.table).by_device.get(device).copied()
    }

    /// Whether the scan must leave this candidate alone: a quiet claim names
    /// its address, or the device it identified as.
    pub fn is_quiet(&self, address: &str, key: Option<&DeviceKey>) -> bool {
        lock(&self.table).by_job.values().any(|hold| {
            hold.claim.quiet
                && (hold.claim.address.as_deref() == Some(address)
                    || key.is_some_and(|k| hold.claim.devices.contains(k)))
        })
    }

    /// Takes every device in the claim for `job`, or none. On a conflict it
    /// returns the device, the job holding it and that job's kind.
    pub(crate) fn insert(
        &self,
        job: JobId,
        kind: &str,
        claim: &DeviceClaim,
    ) -> Result<(), (DeviceKey, JobId, String)> {
        let mut table = lock(&self.table);
        for device in &claim.devices {
            if let Some(holder) = table.by_device.get(device) {
                let kind = table
                    .by_job
                    .get(holder)
                    .map_or_else(String::new, |hold| hold.kind.clone());
                return Err((device.clone(), *holder, kind));
            }
        }
        for device in &claim.devices {
            table.by_device.insert(device.clone(), job);
        }
        table.by_job.insert(
            job,
            Hold {
                kind: kind.to_owned(),
                claim: claim.clone(),
            },
        );
        Ok(())
    }

    /// Frees every device the job held.
    pub(crate) fn release(&self, job: JobId) {
        let mut table = lock(&self.table);
        table.by_job.remove(&job);
        table.by_device.retain(|_, holder| *holder != job);
    }
}
