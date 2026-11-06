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
/// How long the board may stop answering once it has the install (a
/// restart into the slot, through a robot controller, takes about a minute).
const LOST_GRACE: Duration = Duration::from_secs(5 * 60);

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
    let unreadable = |error: serde_json::Error| {
        DriverError::Other(format!("the board's slot list is unreadable: {error}"))
    };
    // One slot per key (`update.slot_count`, `update.slots.<n>`: a list
    // outgrows one Orion status value), else the whole list in one.
    let slots = match keys
        .get("update.slot_count")
        .and_then(|n| n.parse::<usize>().ok())
    {
        Some(count) => (0..count)
            .filter_map(|index| keys.get(&format!("update.slots.{index}")))
            .map(|json| serde_json::from_str(json).map_err(unreadable))
            .collect::<Result<Vec<_>, _>>()?,
        None => match keys.get("update.slots") {
            Some(json) if !json.is_empty() => serde_json::from_str(json).map_err(unreadable)?,
            _ => Vec::new(),
        },
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
        progress.step_started(UpdateStep::Transfer);
        // Behind a robot controller the board can't reach this computer:
        // the bundle goes up through the controller's HeliOS first (the
        // first half of the transfer), else the board fetches it from here.
        let peer = atlas_driver::controller_peer::ControllerPeer::from_address(&device.address);
        let uploaded = peer.is_some();
        let url = match peer {
            Some(peer) => {
                let sink = progress.clone();
                atlas_driver::peer_upload::upload(&peer, &install.bundle, move |done| {
                    sink.step_progress(UpdateStep::Transfer, done / 2.0);
                })
                .await?
            }
            None => {
                let bundles = self.bundles.as_ref().ok_or_else(|| {
                    DriverError::Unsupported("Atlas isn't serving files to boards".into())
                })?;
                bundles.url_for(&install.bundle, peer_address(&device.address).await)?
            }
        };
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
        // When the board stopped answering, after it took the install.
        let mut lost: Option<tokio::time::Instant> = None;
        loop {
            tokio::time::sleep(self.poll).await;
            let keys = match update_status(self.transport.as_ref(), &self.node).await {
                Ok(keys) => keys,
                // The board has the install: losing it now is it busy or
                // restarting into the slot. Never `Unreachable`, which
                // makes the caller install it again the other way in.
                Err(DriverError::Unreachable(reason)) => {
                    let since = *lost.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() >= LOST_GRACE {
                        return Err(DriverError::StepFailed {
                            step: "Installing the slot".into(),
                            message: format!(
                                "the board stopped answering ({reason}); check its slots \
                                 before installing again"
                            ),
                        });
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            let state = keys.get(update_action::KEY_STATE).map(String::as_str);
            // Back after going away: a restart, which ends the install.
            if lost.take().is_some()
                && install.boot
                && !matches!(
                    state,
                    Some(update_action::STATE_STAGING | update_action::STATE_REBOOTING)
                )
            {
                let active = keys.get("update.slot_active").map(String::as_str);
                if active == Some(set.running.as_str()) {
                    return Err(DriverError::StepFailed {
                        step: "Restarting into the slot".into(),
                        message: format!(
                            "the board restarted on its old slot ({}); the new one is \
                             installed but didn't come up",
                            set.running
                        ),
                    });
                }
                progress.step_started(UpdateStep::Apply);
                break;
            }
            match state {
                Some(update_action::STATE_STAGING) | None => {
                    let permille: f32 = keys
                        .get(update_action::KEY_PROGRESS)
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(0.0);
                    let done = permille / 1000.0;
                    let done = if uploaded { 0.5 + done / 2.0 } else { done };
                    progress.step_progress(UpdateStep::Transfer, done);
                }
                // The slot is in: from here the board has changed.
                Some(update_action::STATE_STAGED) if !install.boot => {
                    progress.step_started(UpdateStep::Apply);
                    break;
                }
                Some(update_action::STATE_REBOOTING) => {
                    progress.step_started(UpdateStep::Apply);
                    break;
                }
                // The restart was refused: installed, but still on the old slot.
                Some(update_action::STATE_STAGED)
                    if keys
                        .get(update_action::KEY_ERROR)
                        .is_some_and(|e| !e.is_empty()) =>
                {
                    progress.step_started(UpdateStep::Apply);
                    return Err(DriverError::StepFailed {
                        step: "Restarting into the slot".into(),
                        message: keys[update_action::KEY_ERROR].clone(),
                    });
                }
                Some(update_action::STATE_CANCELLED) => return Err(DriverError::Cancelled),
                Some(update_action::STATE_ERROR) => {
                    let message = keys
                        .get(update_action::KEY_ERROR)
                        .cloned()
                        .unwrap_or_else(|| "the board gave no reason".into());
                    // The board couldn't fetch the file from this computer
                    // (a firewall, the wrong address): nothing changed on
                    // it, so the other way in (SSH, which sends the file
                    // itself) may still work.
                    if atlas_driver::download_failed(&message) {
                        return Err(DriverError::Unreachable(format!(
                            "the board couldn't download the slot from this computer ({message})"
                        )));
                    }
                    return Err(DriverError::StepFailed {
                        step: "Installing the slot".into(),
                        message,
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
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use atlas_driver::{Artifact, DeviceKey, DeviceMode, LinkId};
    use orion_control_plane::{
        ActionRequest, ActionResult, ActionState, NodeRecord, StatusEntry, StatusQuery,
        StatusSubject,
    };

    /// One `update.*` answer: the keys, or `None` for a lost connection.
    type Answer = Option<Vec<(&'static str, &'static str)>>;

    /// A board whose `update.*` answers follow a script (the last repeats).
    struct Board(Mutex<VecDeque<Answer>>);

    #[async_trait]
    impl OrionTransport for Board {
        async fn nodes(&self) -> Result<Vec<NodeRecord>, DriverError> {
            Ok(Vec::new())
        }
        async fn status(&self, _: StatusQuery) -> Result<Vec<StatusEntry>, DriverError> {
            let mut script = self.0.lock().unwrap();
            let answer = if script.len() > 1 {
                script.pop_front().unwrap()
            } else {
                script[0].clone()
            };
            let keys = answer.ok_or_else(|| DriverError::Unreachable("reset by peer".into()))?;
            Ok(keys
                .into_iter()
                .map(|(key, value)| {
                    StatusEntry::new(
                        StatusSubject::Node(NodeId::new("raze-1")),
                        key,
                        TypedConfigValue::String(value.into()),
                    )
                })
                .collect())
        }
        async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError> {
            Ok(ActionResult {
                action_id: request.action_id,
                target: request.target,
                name: request.name,
                state: ActionState::Succeeded,
                output: BTreeMap::new(),
                handled_by: NodeId::new("raze-1"),
                requested_by: "atlas".into(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
        }
        async fn query_action(&self, _: &str) -> Result<Option<ActionResult>, DriverError> {
            Ok(None)
        }
    }

    struct Bundles;
    impl BundleHost for Bundles {
        fn url_for(
            &self,
            artifact: &Artifact,
            _: Option<std::net::IpAddr>,
        ) -> Result<String, DriverError> {
            Ok(format!("http://atlas:7700/{}", artifact.name))
        }
    }

    fn on(state: &'static str, active: &'static str) -> Answer {
        Some(vec![
            ("update.state", state),
            ("update.layout", "slots"),
            ("update.slot_active", active),
            ("update.free_bytes", "9000000000"),
            ("update.reserve_bytes", "0"),
            (
                "update.slots",
                r#"[{"id":"old","flags":["running","default"]}]"#,
            ),
        ])
    }

    async fn install_through(script: Vec<Answer>) -> Result<(), DriverError> {
        let slots = OrionSlots {
            transport: Arc::new(Board(Mutex::new(script.into()))),
            bundles: Some(Arc::new(Bundles)),
            node: NodeId::new("raze-1"),
            poll: Duration::from_secs(1),
        };
        let device = Identity {
            key: DeviceKey::new("raze", "1"),
            model: "Raze".into(),
            mode: DeviceMode::Normal,
            versions: Default::default(),
            name: None,
            link: LinkId("controller".into()),
            address: "http://127.0.0.1:5899/".into(),
            attributes: Default::default(),
        };
        let install = SlotInstall {
            bundle: Artifact {
                name: "new.img.xz".into(),
                path: "new.img.xz".into(),
                sha256: "00".into(),
                size_bytes: 1,
            },
            pin: false,
            boot: true,
        };
        slots
            .install(&device, &install, &ProgressSink::discard())
            .await
    }

    #[tokio::test(start_paused = true)]
    async fn a_board_that_goes_away_with_the_install_restarted_into_it() {
        // Preflight, staging, the restart (no answer), then the new slot.
        let script = vec![on("idle", "old"), on("staging", "old"), None, None];
        let mut back = script.clone();
        back.push(on("confirmed", "new"));
        assert!(install_through(back).await.is_ok());

        // Back on the old slot: the new one didn't come up.
        let mut old = script.clone();
        old.push(on("confirmed", "old"));
        let error = install_through(old).await.unwrap_err();
        assert!(matches!(error, DriverError::StepFailed { .. }), "{error:?}");

        // Gone for good: a failed step, never `Unreachable`, so nothing
        // installs it a second time the other way in.
        let error = install_through(script).await.unwrap_err();
        assert!(matches!(error, DriverError::StepFailed { .. }), "{error:?}");
    }

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

        // One slot per key wins over the whole list (which a long list
        // leaves out).
        let mut keys = keys;
        keys.remove("update.slots");
        keys.insert("update.slot_count".into(), "2".into());
        keys.insert(
            "update.slots.0".into(),
            r#"{"id":"pv-a","flags":["running"]}"#.into(),
        );
        keys.insert("update.slots.1".into(), r#"{"id":"pv-c"}"#.into());
        let set = slot_set(&keys).unwrap().unwrap();
        let ids: Vec<&str> = set.slots.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["pv-a", "pv-c"]);
    }
}
