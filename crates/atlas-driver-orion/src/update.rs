//! A/B updates through Orion (docs/ota.md): Orion carries the intent and
//! progress, the device pulls the disk image from Atlas, and its package's
//! writer stages, trial-boots, and confirms.
//!
//! The `update` action is asynchronous (Orion `docs/device-agent.md`): it
//! succeeds with `phase = "staging"` once the board's agent has started the
//! download and stage. Everything after that is read from durable state: the
//! `update.*` status keys the agent keeps published (`staging` -> `staged` ->
//! `rebooting`, then on the new boot `trying` -> `confirmed` |
//! `rolled-back`), and the node's boot id, never from an action result.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::attributes::normalize_board_serial;
use atlas_driver::{
    CancellationToken, Concurrency, DriverError, Identity, ProgressSink, ReleaseRef,
    UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use orion_control_plane::{StatusQuery, StatusSubject, TypedConfigValue, update_action};
use orion_core::NodeId;

use crate::actions::{request, run_and_wait};
use crate::transport::{BundleHost, OrionTransport};

/// The `update` action only starts the stage.
const START_TIMEOUT: Duration = Duration::from_secs(60);
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
    poll: Duration,
}

/// How the stage ended, as the `update.*` keys tell it.
enum Staged {
    /// The board restarts into the new slot (or already did).
    Restarting,
    Failed(String),
    Cancelled,
}

impl OrionUpdate {
    pub(crate) fn new(
        transport: Arc<dyn OrionTransport>,
        bundles: Arc<dyn BundleHost>,
        node: NodeId,
        board_serial: String,
        poll: Duration,
    ) -> Self {
        Self {
            transport,
            bundles,
            node,
            board_serial,
            poll,
        }
    }

    /// The durable `update.*` keys the device's agent republishes.
    async fn update_status(&self, node: &NodeId) -> Result<HashMap<String, String>, DriverError> {
        let entries = self
            .transport
            .status(StatusQuery {
                subject: Some(StatusSubject::Node(node.clone())),
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
                    TypedConfigValue::F64(x) => x.to_string(),
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

    /// Follows the stage through the `update.*` keys until the board
    /// restarts, reporting the download as Transfer and the slot write as
    /// Apply (the agent's progress: 0-500 and 500-1000 per mille). Atlas's
    /// cancel sends `update.cancel` while it can still stop the stage.
    async fn follow_stage(
        &self,
        boot_before: &str,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<Staged, DriverError> {
        let mut applying = false;
        let deadline = tokio::time::Instant::now() + STAGE_TIMEOUT;
        loop {
            if cancel.is_cancelled() {
                let result = run_and_wait(
                    self.transport.as_ref(),
                    request(&self.node, "update.cancel", BTreeMap::new()),
                    START_TIMEOUT,
                    &|_| {},
                )
                .await;
                return match result {
                    Ok(_) => Ok(Staged::Cancelled),
                    // Too late (the restart began): carry on to the outcome.
                    Err(error) => {
                        progress.log(format!("couldn't cancel: {error}"));
                        Ok(Staged::Restarting)
                    }
                };
            }
            // A new boot id: the board restarted (keys of the old boot may
            // still be around, so this is checked first).
            if let Ok(Some((_, boot))) = self.boot_id().await
                && boot != boot_before
            {
                return Ok(Staged::Restarting);
            }
            let status = self.update_status(&self.node).await.unwrap_or_default();
            let per_mille: u16 = status
                .get(update_action::KEY_PROGRESS)
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            match status.get(update_action::KEY_STATE).map(String::as_str) {
                Some(update_action::STATE_STAGING) => {
                    let fraction = f32::from(per_mille.min(1000)) / 1000.0;
                    if fraction < 0.5 {
                        progress.step_progress(UpdateStep::Transfer, fraction * 2.0);
                    } else {
                        if !applying {
                            applying = true;
                            progress.step_progress(UpdateStep::Transfer, 1.0);
                            progress.step_started(UpdateStep::Apply);
                        }
                        progress.step_progress(UpdateStep::Apply, (fraction - 0.5) * 2.0);
                    }
                }
                Some(update_action::STATE_STAGED) => {
                    if !applying {
                        applying = true;
                        progress.step_started(UpdateStep::Apply);
                    }
                    progress.step_progress(UpdateStep::Apply, 1.0);
                    // Staged with an error: the restart was refused (for
                    // example the OS's pre-reboot hook); it stays staged.
                    if let Some(error) = status.get(update_action::KEY_ERROR)
                        && !error.is_empty()
                    {
                        return Ok(Staged::Failed(error.clone()));
                    }
                }
                Some(
                    update_action::STATE_REBOOTING
                    | update_action::STATE_TRYING
                    | update_action::STATE_CONFIRMED
                    | update_action::STATE_ROLLED_BACK,
                ) => return Ok(Staged::Restarting),
                Some(update_action::STATE_ERROR) => {
                    return Ok(Staged::Failed(
                        status
                            .get(update_action::KEY_ERROR)
                            .filter(|error| !error.is_empty())
                            .cloned()
                            .unwrap_or_else(|| "the board couldn't stage the image".into()),
                    ));
                }
                Some(update_action::STATE_CANCELLED) => return Ok(Staged::Cancelled),
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(DriverError::Unreachable(
                    "the board didn't finish staging the image in time".into(),
                ));
            }
            tokio::time::sleep(self.poll).await;
        }
    }
}

/// The board's IP address from its identity address (a URL such as
/// `http://172.31.250.1:5899/...`, or `host:port`), resolving a name. The
/// bundle host uses it to pick the local address that routes to the board.
async fn peer_address(address: &str) -> Option<IpAddr> {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(bracketed) = authority.strip_prefix('[') {
        return bracketed.split(']').next()?.parse().ok();
    }
    if let Ok(ip) = authority.parse() {
        return Some(ip);
    }
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    if host.is_empty() {
        return None;
    }
    if let Ok(ip) = host.parse() {
        return Some(ip);
    }
    let lookup = tokio::net::lookup_host((host, 0));
    tokio::time::timeout(Duration::from_secs(5), lookup)
        .await
        .ok()?
        .ok()?
        .next()
        .map(|socket| socket.ip())
}

#[async_trait]
impl UpdateCapability for OrionUpdate {
    fn plan(&self, _device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        let artifact = release
            .artifact
            .as_ref()
            .ok_or_else(|| DriverError::Incompatible("choose an image to install".into()))?;
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
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let artifact = release
            .artifact
            .as_ref()
            .ok_or_else(|| DriverError::Incompatible("choose an image to install".into()))?;

        progress.step_started(UpdateStep::Preflight);
        let status = self.update_status(&self.node).await?;
        if let Some(
            state @ (update_action::STATE_STAGING
            | update_action::STATE_REBOOTING
            | update_action::STATE_TRYING),
        ) = status.get(update_action::KEY_STATE).map(String::as_str)
        {
            return Err(DriverError::Incompatible(format!(
                "the board is already updating ({state}); wait for it to finish"
            )));
        }
        let (_, boot_before) = self
            .boot_id()
            .await?
            .ok_or_else(|| DriverError::Unreachable("the board isn't reporting to Orion".into()))?;
        let peer = peer_address(&device.address).await;
        let url = self.bundles.url_for(artifact, peer)?;
        if cancel.is_cancelled() {
            return Err(DriverError::Cancelled);
        }

        // Transfer and Apply both run on the device: the action only starts
        // them, the keys tell how far they got.
        progress.step_started(UpdateStep::Transfer);
        let args = BTreeMap::from([
            (
                update_action::ARG_IMAGE_URL.to_string(),
                TypedConfigValue::String(url),
            ),
            (
                update_action::ARG_SHA256.to_string(),
                TypedConfigValue::String(artifact.sha256.clone()),
            ),
            (
                update_action::ARG_SIZE.to_string(),
                TypedConfigValue::UInt(artifact.size_bytes),
            ),
        ]);
        run_and_wait(
            self.transport.as_ref(),
            request(&self.node, "update", args),
            START_TIMEOUT,
            &|_| {},
        )
        .await?;
        match self.follow_stage(&boot_before, progress, cancel).await? {
            Staged::Restarting => {}
            Staged::Cancelled => return Err(DriverError::Cancelled),
            Staged::Failed(message) => {
                return Err(DriverError::StepFailed {
                    step: "Writing the spare slot".into(),
                    message,
                });
            }
        }
        progress.step_progress(UpdateStep::Apply, 1.0);
        progress.log("staged; the board is restarting into it".to_string());

        // Past this point the board is restarting: no cancelling.
        progress.step_started(UpdateStep::Reboot);
        let deadline = tokio::time::Instant::now() + REBOOT_TIMEOUT;
        let (node, boot_after) = loop {
            if let Ok(Some((node, boot))) = self.boot_id().await
                && boot != boot_before
            {
                break (node, boot);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(UpdateOutcome::NeedsRecovery {
                    reason: "the board didn't come back after restarting into the update".into(),
                });
            }
            tokio::time::sleep(self.poll).await;
        };

        progress.step_started(UpdateStep::Confirm);
        let deadline = tokio::time::Instant::now() + CONFIRM_TIMEOUT;
        loop {
            let status = self.update_status(&node).await.unwrap_or_default();
            // Keys left from the boot before the restart don't count.
            let this_boot = status
                .get(update_action::KEY_BOOT_ID)
                .is_none_or(|boot| boot.is_empty() || *boot == boot_after);
            match status.get(update_action::KEY_STATE).map(String::as_str) {
                Some(update_action::STATE_CONFIRMED) if this_boot => {
                    let active = status
                        .get(update_action::KEY_VERSION_ACTIVE)
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
                Some(update_action::STATE_ROLLED_BACK) if this_boot => {
                    return Ok(UpdateOutcome::RolledBack {
                        reason: status
                            .get(update_action::KEY_ERROR)
                            .filter(|error| !error.is_empty())
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
            tokio::time::sleep(self.poll).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::peer_address;

    #[tokio::test]
    async fn the_board_address_comes_from_its_identity_address() {
        let ip = |text: &str| text.parse::<std::net::IpAddr>().ok();
        assert_eq!(
            peer_address("http://172.31.250.1:5899/.well-known/pd-device").await,
            ip("172.31.250.1")
        );
        assert_eq!(peer_address("10.0.0.7:22").await, ip("10.0.0.7"));
        assert_eq!(peer_address("10.0.0.7").await, ip("10.0.0.7"));
        assert_eq!(peer_address("http://[fd00::2]:80/").await, ip("fd00::2"));
        assert_eq!(peer_address("fd00::2").await, ip("fd00::2"));
        assert!(
            peer_address("http://localhost:1/")
                .await
                .is_some_and(|ip| ip.is_loopback())
        );
        assert_eq!(peer_address("http:///path").await, None);
    }
}
