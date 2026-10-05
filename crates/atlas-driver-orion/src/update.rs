//! A/B updates through Orion (docs/ota.md): Orion carries the intent and
//! progress, the device pulls the bundle from Atlas, and its package's
//! writer stages, trial-boots, and confirms. The outcome is read from
//! durable state after the reboot (`update.*` status keys and the node's
//! boot id), never from the action result, which doesn't survive a reboot.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::attributes::normalize_board_serial;
use atlas_driver::{
    CancellationToken, Concurrency, DriverError, Identity, ProgressSink, ReleaseRef,
    UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use orion_control_plane::{StatusQuery, StatusSubject, TypedConfigValue};
use orion_core::NodeId;

use crate::actions::{request, run_and_wait};
use crate::transport::{BundleHost, OrionTransport};

const POLL: Duration = Duration::from_secs(3);
/// Download + write of a full slot on a slow link.
const STAGE_TIMEOUT: Duration = Duration::from_secs(45 * 60);
/// From "rebooting" to the node reporting a new boot id.
const REBOOT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// From the new boot to `confirmed` or `rolled-back`.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub(crate) struct OrionUpdate {
    transport: Arc<dyn OrionTransport>,
    bundles: Arc<dyn BundleHost>,
    node: NodeId,
    board_serial: String,
}

impl OrionUpdate {
    pub(crate) fn new(
        transport: Arc<dyn OrionTransport>,
        bundles: Arc<dyn BundleHost>,
        node: NodeId,
        board_serial: String,
    ) -> Self {
        Self {
            transport,
            bundles,
            node,
            board_serial,
        }
    }

    /// The durable `update.*` keys the device's agent republishes.
    async fn update_status(&self) -> Result<HashMap<String, String>, DriverError> {
        let entries = self
            .transport
            .status(StatusQuery {
                subject: Some(StatusSubject::Node(self.node.clone())),
                key_prefix: Some("update.".into()),
            })
            .await?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let value = match entry.value {
                    TypedConfigValue::String(text) => text,
                    TypedConfigValue::UInt(n) => n.to_string(),
                    TypedConfigValue::Int(n) => n.to_string(),
                    TypedConfigValue::Bool(b) => b.to_string(),
                    TypedConfigValue::Bytes(_) => return None,
                };
                Some((entry.key, value))
            })
            .collect())
    }

    /// The node's current boot id, found by board serial: after a reboot the
    /// node may come back under the same id or re-enroll under a new one.
    async fn boot_id(&self) -> Result<Option<(NodeId, String)>, DriverError> {
        Ok(self
            .transport
            .nodes()
            .await?
            .into_iter()
            .find_map(|record| {
                let host = record.host?;
                let serial = normalize_board_serial(host.board_serial.as_deref()?)?;
                (serial == self.board_serial).then_some((record.node_id, host.boot_id?))
            }))
    }
}

#[async_trait]
impl UpdateCapability for OrionUpdate {
    fn plan(&self, _device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        let artifact = release.artifact.as_ref().ok_or_else(|| {
            DriverError::Incompatible("choose an update bundle (.pdupdate) to install".into())
        })?;
        Ok(UpdatePlan {
            steps: vec![
                UpdateStep::Preflight,
                UpdateStep::Transfer,
                UpdateStep::Apply,
                UpdateStep::Reboot,
                UpdateStep::Confirm,
            ],
            concurrency: Concurrency::Parallel,
            summary: format!(
                "Update to {} with {}: the board writes its spare slot, restarts into it, and keeps it once it's healthy",
                release.version, artifact.name
            ),
        })
    }

    async fn run(
        &self,
        _device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let artifact = release
            .artifact
            .as_ref()
            .ok_or_else(|| DriverError::Incompatible("no update bundle".into()))?;

        progress.step_started(UpdateStep::Preflight);
        let status = self.update_status().await?;
        if let Some(state @ ("staging" | "trying")) = status.get("update.state").map(String::as_str)
        {
            return Err(DriverError::Incompatible(format!(
                "the board is already updating ({state}); wait for it to finish"
            )));
        }
        let (_, boot_before) = self
            .boot_id()
            .await?
            .ok_or_else(|| DriverError::Unreachable("the board isn't reporting to Orion".into()))?;
        let url = self.bundles.url_for(artifact)?;
        if cancel.is_cancelled() {
            return Err(DriverError::Cancelled);
        }

        // Transfer and Apply both run on the device: the first half of its
        // progress is the download, the second half the slot write.
        progress.step_started(UpdateStep::Transfer);
        let args = BTreeMap::from([
            ("bundle_url".to_string(), TypedConfigValue::String(url)),
            (
                "sha256".to_string(),
                TypedConfigValue::String(artifact.sha256.clone()),
            ),
            (
                "size".to_string(),
                TypedConfigValue::UInt(artifact.size_bytes),
            ),
        ]);
        let applying = AtomicBool::new(false);
        let report = |per_mille: u16| {
            let fraction = f32::from(per_mille) / 1000.0;
            if fraction < 0.5 {
                progress.step_progress(UpdateStep::Transfer, fraction * 2.0);
            } else {
                if !applying.swap(true, Ordering::Relaxed) {
                    progress.step_started(UpdateStep::Apply);
                }
                progress.step_progress(UpdateStep::Apply, (fraction - 0.5) * 2.0);
            }
        };
        let staged = run_and_wait(
            self.transport.as_ref(),
            request(&self.node, "update", args),
            STAGE_TIMEOUT,
            &report,
        )
        .await?;
        if !applying.load(Ordering::Relaxed) {
            progress.step_started(UpdateStep::Apply);
        }
        progress.step_progress(UpdateStep::Apply, 1.0);
        if let Some(TypedConfigValue::String(version)) = staged.output.get("version_staged") {
            progress.log(format!("staged {version}; the board is restarting into it"));
        }

        // Past this point the board is restarting: no cancelling.
        progress.step_started(UpdateStep::Reboot);
        let deadline = tokio::time::Instant::now() + REBOOT_TIMEOUT;
        loop {
            if let Ok(Some((_, boot))) = self.boot_id().await
                && boot != boot_before
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(UpdateOutcome::NeedsRecovery {
                    reason: "the board didn't come back after restarting into the update".into(),
                });
            }
            tokio::time::sleep(POLL).await;
        }

        progress.step_started(UpdateStep::Confirm);
        let deadline = tokio::time::Instant::now() + CONFIRM_TIMEOUT;
        loop {
            let status = self.update_status().await.unwrap_or_default();
            match status.get("update.state").map(String::as_str) {
                Some("confirmed") => {
                    let active = status
                        .get("update.version_active")
                        .cloned()
                        .unwrap_or_default();
                    return Ok(if active == release.version {
                        UpdateOutcome::Verified { version: active }
                    } else {
                        UpdateOutcome::RolledBack {
                            reason: format!(
                                "the board runs {active} instead of {}",
                                release.version
                            ),
                        }
                    });
                }
                Some("rolled-back") => {
                    return Ok(UpdateOutcome::RolledBack {
                        reason: status
                            .get("update.error")
                            .cloned()
                            .unwrap_or_else(|| "the new version didn't confirm itself".into()),
                    });
                }
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(UpdateOutcome::RolledBack {
                    reason: "the board never confirmed the new version; it returns to the old one on its next restart".into(),
                });
            }
            tokio::time::sleep(POLL).await;
        }
    }
}
