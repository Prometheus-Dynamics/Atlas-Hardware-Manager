//! A slots board through Orion (board-agent): the slots and the free space
//! from the `update.*` keys its agent publishes (`update.layout = slots`,
//! `update.slots` as JSON), the `slot.*` actions, and installing a bundle
//! with the `update` action (`apply: false` to install only), the board
//! pulling it from Atlas's bundle host as it pulls an A/B image.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    DriverError, Identity, ProgressSink, SlotCommand, SlotInstall, SlotSet, SlotsCapability,
    UpdateStep,
};
use orion_control_plane::{TypedConfigValue, update_action};
use orion_core::NodeId;

use crate::actions::{request, run_and_wait};
use crate::transport::{BundleHost, OrionTransport};
use crate::update::{peer_address, update_status};

/// The action board-agent claims on a slots-capable build.
pub(crate) const SLOT_SET_DEFAULT: &str = "slot.set-default";
/// set-default, remove and pin answer once the writer has; boot-once
/// answers before the restart.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// Download and install of a slot on a slow link.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);

pub(crate) struct OrionSlots {
    pub(crate) transport: Arc<dyn OrionTransport>,
    pub(crate) bundles: Option<Arc<dyn BundleHost>>,
    pub(crate) node: NodeId,
    pub(crate) poll: Duration,
}

/// The slot set the keys tell, or `None` on an A/B board.
pub(crate) fn slot_set(keys: &HashMap<String, String>) -> Result<Option<SlotSet>, DriverError> {
    // Nothing published yet: the board is still starting, not an A/B board.
    if !keys.contains_key(update_action::KEY_STATE) {
        return Err(DriverError::Unreachable(
            "the board hasn't reported its slots yet (still starting?); try again in a moment"
                .into(),
        ));
    }
    if keys.get("update.layout").map(String::as_str) != Some("slots") {
        return Ok(None);
    }
    let get = |key: &str| keys.get(key).cloned().unwrap_or_default();
    let bytes = |key: &str| get(key).parse().unwrap_or(0);
    let slots = match keys.get("update.slots") {
        Some(json) if !json.is_empty() => serde_json::from_str(json).map_err(|error| {
            DriverError::Other(format!("the board's slot list is unreadable: {error}"))
        })?,
        _ => Vec::new(),
    };
    Ok(Some(SlotSet {
        running: get(update_action::KEY_SLOT_ACTIVE),
        default: get("update.slot_default"),
        previous: get("update.slot_previous"),
        trial: get("update.slot_trial"),
        free_bytes: bytes("update.free_bytes"),
        reserve_bytes: bytes("update.reserve_bytes"),
        slots,
    }))
}

#[async_trait]
impl SlotsCapability for OrionSlots {
    fn preferred(&self) -> bool {
        true
    }

    async fn slots(&self, _device: &Identity) -> Result<Option<SlotSet>, DriverError> {
        slot_set(&update_status(self.transport.as_ref(), &self.node).await?)
    }

    async fn command(&self, _device: &Identity, command: &SlotCommand) -> Result<(), DriverError> {
        let mut args = BTreeMap::from([(
            "id".to_string(),
            TypedConfigValue::String(command.id().to_string()),
        )]);
        let name = match command {
            SlotCommand::BootOnce { .. } => "slot.boot-once",
            SlotCommand::SetDefault { .. } => SLOT_SET_DEFAULT,
            SlotCommand::Remove { .. } => "slot.remove",
            SlotCommand::Pin { pinned, .. } => {
                args.insert("pinned".into(), TypedConfigValue::Bool(*pinned));
                "slot.pin"
            }
        };
        run_and_wait(
            self.transport.as_ref(),
            request(&self.node, name, args),
            COMMAND_TIMEOUT,
            &|_| {},
        )
        .await
        .map(|_| ())
        .map_err(|error| match error {
            DriverError::Other(reason) => DriverError::Other(atlas_driver::refusal(&reason)),
            other => other,
        })
    }

    async fn install(
        &self,
        device: &Identity,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        let bundles = self.bundles.as_ref().ok_or_else(|| {
            DriverError::Unsupported("Atlas isn't serving files to boards".into())
        })?;
        progress.step_started(UpdateStep::Preflight);
        let keys = update_status(self.transport.as_ref(), &self.node).await?;
        let set = slot_set(&keys)?
            .ok_or_else(|| DriverError::Incompatible("this board has no slots".into()))?;
        if let Some(state @ (update_action::STATE_STAGING | update_action::STATE_REBOOTING)) =
            keys.get(update_action::KEY_STATE).map(String::as_str)
        {
            return Err(DriverError::Incompatible(format!(
                "the board is busy ({state}); wait for it to finish"
            )));
        }
        if install.bundle.size_bytes > set.room() {
            return Err(DriverError::Incompatible(format!(
                "the board has {} MiB of room; the slot needs {} MiB (remove one first)",
                set.room() >> 20,
                install.bundle.size_bytes >> 20
            )));
        }
        let url = bundles.url_for(&install.bundle, peer_address(&device.address).await)?;
        progress.step_started(UpdateStep::Transfer);
        let args = BTreeMap::from([
            (
                update_action::ARG_IMAGE_URL.to_string(),
                TypedConfigValue::String(url),
            ),
            (
                update_action::ARG_SHA256.to_string(),
                TypedConfigValue::String(install.bundle.sha256.clone()),
            ),
            (
                update_action::ARG_SIZE.to_string(),
                TypedConfigValue::UInt(install.bundle.size_bytes),
            ),
            ("apply".to_string(), TypedConfigValue::Bool(install.boot)),
            ("pin".to_string(), TypedConfigValue::Bool(install.pin)),
        ]);
        run_and_wait(
            self.transport.as_ref(),
            request(&self.node, "update", args),
            START_TIMEOUT,
            &|_| {},
        )
        .await?;
        // The download and install run on the board; the keys tell how far.
        let deadline = tokio::time::Instant::now() + INSTALL_TIMEOUT;
        loop {
            tokio::time::sleep(self.poll).await;
            let keys = update_status(self.transport.as_ref(), &self.node).await?;
            let state = keys.get(update_action::KEY_STATE).map(String::as_str);
            match state {
                Some(update_action::STATE_STAGING) | None => {
                    let permille: f32 = keys
                        .get(update_action::KEY_PROGRESS)
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(0.0);
                    progress.step_progress(UpdateStep::Transfer, permille / 1000.0);
                }
                Some(update_action::STATE_STAGED) if !install.boot => break,
                Some(update_action::STATE_REBOOTING) => break,
                // The restart was refused: installed, but still on the old slot.
                Some(update_action::STATE_STAGED)
                    if keys
                        .get(update_action::KEY_ERROR)
                        .is_some_and(|e| !e.is_empty()) =>
                {
                    return Err(DriverError::StepFailed {
                        step: "Restarting into the slot".into(),
                        message: keys[update_action::KEY_ERROR].clone(),
                    });
                }
                Some(update_action::STATE_CANCELLED) => return Err(DriverError::Cancelled),
                Some(update_action::STATE_ERROR) => {
                    return Err(DriverError::StepFailed {
                        step: "Installing the slot".into(),
                        message: keys
                            .get(update_action::KEY_ERROR)
                            .cloned()
                            .unwrap_or_else(|| "the board gave no reason".into()),
                    });
                }
                Some(_) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(DriverError::Other(
                    "the board is still installing the slot; check it again later".into(),
                ));
            }
        }
        progress.step_progress(UpdateStep::Transfer, 1.0);
        progress.log(if install.boot {
            "installed; the board is restarting into it once".to_string()
        } else {
            "installed".to_string()
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ab_board_has_no_slot_set() {
        let keys = HashMap::from([("update.state".to_string(), "idle".to_string())]);
        assert_eq!(slot_set(&keys).unwrap(), None);
        // Nothing published yet is "still starting", not "no slots".
        assert!(matches!(
            slot_set(&HashMap::new()),
            Err(DriverError::Unreachable(_))
        ));
    }

    #[test]
    fn reads_the_published_keys() {
        let keys: HashMap<String, String> = [
            ("update.state", "idle"),
            ("update.layout", "slots"),
            ("update.slot_active", "pv-a"),
            ("update.slot_default", "pv-a"),
            ("update.free_bytes", "5000"),
            ("update.reserve_bytes", "1000"),
            (
                "update.slots",
                r#"[{"id":"pv-a","flags":["running","default"],"good":"yes","display_name":"HeliOS"},{"id":"pv-b","pinned":true}]"#,
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let set = slot_set(&keys).unwrap().unwrap();
        assert_eq!(set.running, "pv-a");
        assert_eq!(set.room(), 4000);
        assert_eq!(set.slots.len(), 2);
        assert!(set.slots[1].pinned);
        assert_eq!(set.slots[0].display_name.as_deref(), Some("HeliOS"));
        assert_eq!(set.slots[1].display_name, None);
    }
}
