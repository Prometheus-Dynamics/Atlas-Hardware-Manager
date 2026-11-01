//! Updates that outlived an earlier Atlas: a flash that goes on without
//! Atlas (Atlas's USB agent on a SystemCore keeps flashing when Atlas closes
//! or crashes) is picked up again after a scan, as a job of its own, so its
//! end reaches the job history.

use std::collections::BTreeMap;

use crate::Atlas;
use crate::jobs::{JobState, ReleaseTarget, UpdateRequest};

impl Atlas {
    pub(crate) fn start_due_resumes(&self) {
        let due: Vec<_> = {
            let mut state = self.inner.state();
            let busy: Vec<_> = state
                .jobs
                .values()
                .filter(|job| job.state == JobState::Running)
                .flat_map(|job| job.devices.iter().map(|device| device.device.clone()))
                .collect();
            let mut due = Vec::new();
            for (key, live) in &state.live {
                if state.resumed.contains(key) || busy.contains(key) {
                    continue;
                }
                let (Some(update), Some(record)) =
                    (live.capabilities.update.as_ref(), state.inventory.get(key))
                else {
                    continue;
                };
                if let Some(release) = update.resumable(&record.identity) {
                    due.push((key.clone(), release));
                }
            }
            for (key, _) in &due {
                state.resumed.insert(key.clone());
            }
            due
        };
        for (key, release) in due {
            let request = UpdateRequest {
                devices: vec![key.clone()],
                releases: BTreeMap::from([(
                    release.family.clone(),
                    ReleaseTarget {
                        version: release.version.clone(),
                        artifact: release.artifact.clone(),
                    },
                )]),
                ..UpdateRequest::default()
            };
            // One that can't start (its image moved) stays a record on the
            // device's side; the next flash there says so.
            let _ = self.start_update(request);
        }
    }
}
