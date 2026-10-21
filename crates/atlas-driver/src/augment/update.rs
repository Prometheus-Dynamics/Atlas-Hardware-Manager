//! Updates from two sources: the one that goes first, and the fallback
//! when it can't start.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    CancellationToken, DriverError, Identity, ProgressSink, ReleaseRef, UpdateCapability,
    UpdateOutcome, UpdatePlan, UpdateStep,
};

/// Two ways to update one device: `first`, and `then` when `first` can't
/// start (it can't reach the device, the device won't let it in, or its
/// checks fail). Only before anything changed, so nothing is written twice:
/// an Orion update falls back to SSH, or SSH to Orion, never midway.
pub(super) struct FallbackUpdate {
    pub(super) first: Arc<dyn UpdateCapability>,
    pub(super) then: Arc<dyn UpdateCapability>,
}

/// Whether an update failed before it changed anything.
fn failed_to_start(error: &DriverError) -> bool {
    match error {
        DriverError::Unreachable(_)
        | DriverError::Incompatible(_)
        | DriverError::Unsupported(_) => true,
        DriverError::StepFailed { step, .. } => step == UpdateStep::Preflight.label(),
        DriverError::Cancelled | DriverError::Other(_) => false,
    }
}

#[async_trait]
impl UpdateCapability for FallbackUpdate {
    fn preferred(&self) -> bool {
        self.first.preferred()
    }

    fn plan(&self, device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        self.first
            .plan(device, release)
            .or_else(|error| self.then.plan(device, release).map_err(|_| error))
    }

    async fn run(
        &self,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let first = match self.first.plan(device, release) {
            Ok(_) => self.first.run(device, release, progress, cancel).await,
            Err(error) => Err(error),
        };
        match first {
            Err(error) if failed_to_start(&error) && !cancel.is_cancelled() => {
                if self.then.plan(device, release).is_err() {
                    return Err(error);
                }
                progress.log(format!("{error}; trying the other way in"));
                self.then
                    .run(device, release, progress, cancel)
                    .await
                    .map_err(|second| match second {
                        // Both failed to start: say why for each.
                        DriverError::Incompatible(why) | DriverError::Unreachable(why)
                            if failed_to_start(&error) =>
                        {
                            DriverError::Incompatible(format!("{error}; and {why}"))
                        }
                        other => other,
                    })
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::{Capabilities, DeviceKey, DeviceMode, LinkId};

    fn device() -> Identity {
        Identity {
            key: DeviceKey::new("raze", "a1"),
            model: "Raze".into(),
            mode: DeviceMode::Normal,
            versions: BTreeMap::new(),
            name: None,
            link: LinkId("mdns".into()),
            address: "x".into(),
            attributes: BTreeMap::new(),
        }
    }

    /// An update that ends as `outcome`, counting its runs.
    struct Updates {
        preferred: bool,
        outcome: fn() -> Result<UpdateOutcome, DriverError>,
        runs: Mutex<usize>,
    }

    #[async_trait]
    impl UpdateCapability for Updates {
        fn preferred(&self) -> bool {
            self.preferred
        }
        fn plan(&self, _: &Identity, _: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
            Ok(UpdatePlan {
                steps: vec![UpdateStep::Apply],
                concurrency: crate::Concurrency::Parallel,
                summary: String::new(),
            })
        }
        async fn run(
            &self,
            _: &Identity,
            _: &ReleaseRef,
            _: &ProgressSink,
            _: &CancellationToken,
        ) -> Result<UpdateOutcome, DriverError> {
            *self.runs.lock().unwrap() += 1;
            (self.outcome)()
        }
    }

    fn updates(
        preferred: bool,
        outcome: fn() -> Result<UpdateOutcome, DriverError>,
    ) -> Arc<Updates> {
        Arc::new(Updates {
            preferred,
            outcome,
            runs: Mutex::new(0),
        })
    }

    async fn update_with(
        own: &Arc<Updates>,
        more: &Arc<Updates>,
    ) -> Result<UpdateOutcome, DriverError> {
        let mut caps = Capabilities {
            update: Some(own.clone()),
            ..Capabilities::default()
        };
        caps.fill_from(Capabilities {
            update: Some(more.clone()),
            ..Capabilities::default()
        });
        let release = ReleaseRef {
            family: crate::Family::new("raze"),
            version: "2".into(),
            artifact: None,
        };
        caps.update
            .unwrap()
            .run(
                &device(),
                &release,
                &ProgressSink::discard(),
                &CancellationToken::new(),
            )
            .await
    }

    fn verified() -> Result<UpdateOutcome, DriverError> {
        Ok(UpdateOutcome::Verified {
            version: "2".into(),
        })
    }

    #[tokio::test]
    async fn orion_updates_first_and_ssh_is_not_touched() {
        let ssh = updates(false, || {
            Err(DriverError::Incompatible(
                "the board doesn't accept your SSH key".into(),
            ))
        });
        let orion = updates(true, verified);
        assert!(update_with(&ssh, &orion).await.is_ok());
        assert_eq!(
            (*ssh.runs.lock().unwrap(), *orion.runs.lock().unwrap()),
            (0, 1)
        );
    }

    #[tokio::test]
    async fn a_rejected_ssh_key_falls_back_to_orion() {
        let ssh = updates(false, || {
            Err(DriverError::Incompatible(
                "the board doesn't accept your SSH key".into(),
            ))
        });
        let orion = updates(false, verified);
        assert!(update_with(&ssh, &orion).await.is_ok());
        assert_eq!(
            (*ssh.runs.lock().unwrap(), *orion.runs.lock().unwrap()),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn an_unreachable_orion_falls_back_to_ssh() {
        let ssh = updates(false, verified);
        let orion = updates(true, || Err(DriverError::Unreachable("no node".into())));
        assert!(update_with(&ssh, &orion).await.is_ok());
        assert_eq!(
            (*ssh.runs.lock().unwrap(), *orion.runs.lock().unwrap()),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn a_failure_midway_is_never_retried_the_other_way() {
        let ssh = updates(false, verified);
        let orion = updates(true, || {
            Err(DriverError::StepFailed {
                step: "apply".into(),
                message: "x".into(),
            })
        });
        assert!(update_with(&ssh, &orion).await.is_err());
        assert_eq!(*ssh.runs.lock().unwrap(), 0);
    }
}
