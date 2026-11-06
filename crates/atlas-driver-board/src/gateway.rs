//! A board behind a robot controller, through the controller's HeliOS
//! gateway: `/v1/peers/<node>/<path>` passes the board's own HeliOS API
//! (`/v1/<path>`), so the board gets Restart (`system/reboot`) and its slots
//! (`update/status`, `update/slots/switch` and the writer's
//! `update/slots/<set-default|remove|pin|unpin>`, `update/uploads` +
//! `update/apply`) without SSH. Orion, when its tunnel reaches the board,
//! goes first.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atlas_driver::controller_peer::ControllerPeer;
use atlas_driver::{
    ActionsCapability, Capabilities, DeviceAction, DriverError, Identity, ProgressSink, Slot,
    SlotCommand, SlotInstall, SlotSet, SlotsCapability, UpdateStep,
};
use serde_json::{Value, json};

use crate::contract::BoardIdentity;
use crate::gateway_metrics::GatewayTelemetry;
use crate::ssh_actions::REBOOT_ACTION;

/// How long the board's writer may stage an image before Atlas stops
/// waiting.
const STAGE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const STAGE_POLL: Duration = Duration::from_secs(2);
/// How long a board may take to restart into a new slot and answer again.
const BOOT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const BOOT_POLL: Duration = Duration::from_secs(3);
/// How long a command through the gateway may take to answer: HeliOS holds
/// `update/apply` until the writer has checked the image (up to 15 s), and
/// the writer's slot commands take seconds. The driver's client is meant
/// for quick reads (2 s for the whole request).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// The board's HeliOS API through the controller.
#[derive(Clone)]
pub(crate) struct Gateway {
    pub(crate) http: reqwest::Client,
    pub(crate) peer: ControllerPeer,
}

impl Gateway {
    fn url(&self, path: &str) -> String {
        format!(
            "http://{}:{}/v1/peers/{}/{path}",
            self.peer.host, self.peer.port, self.peer.node
        )
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, DriverError> {
        let response = request.send().await.map_err(|error| {
            DriverError::Unreachable(format!("the board through {}: {error}", self.peer.host))
        })?;
        let response = atlas_driver::util::answered(response)
            .await
            .map_err(|words| DriverError::Other(format!("the board's HeliOS {words}")))?;
        Ok(response.json().await.unwrap_or(Value::Null))
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Value, DriverError> {
        self.send(self.http.get(self.url(path))).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, DriverError> {
        let request = self.http.post(self.url(path)).json(&body);
        self.send(request.timeout(ANSWER_TIMEOUT)).await
    }
}

/// What a board behind a controller gets through the gateway: Restart, its
/// readings, and its slots when it has them.
pub(crate) fn capabilities(
    http: reqwest::Client,
    peer: ControllerPeer,
    reported: &BoardIdentity,
) -> Capabilities {
    let gateway = Gateway { http, peer };
    let slots = reported
        .update_methods
        .iter()
        .any(|method| method == crate::slots::SLOTS_METHOD)
        .then(|| Arc::new(GatewaySlots(gateway.clone())) as Arc<dyn SlotsCapability>);
    Capabilities {
        telemetry: Some(Arc::new(GatewayTelemetry(gateway.clone()))),
        actions: Some(Arc::new(GatewayActions(gateway))),
        slots,
        ..Capabilities::default()
    }
}

pub(crate) struct GatewayActions(pub(crate) Gateway);

#[async_trait]
impl ActionsCapability for GatewayActions {
    fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
        vec![DeviceAction {
            id: REBOOT_ACTION.into(),
            label: "Restart".into(),
            destructive: false,
        }]
    }

    async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
        match action_id {
            REBOOT_ACTION => self.0.post("system/reboot", json!({})).await.map(|_| ()),
            other => Err(DriverError::Unsupported(other.into())),
        }
    }
}

pub(crate) struct GatewaySlots(pub(crate) Gateway);

/// A board whose HeliOS has no such route (older than the slot commands):
/// its own "no route for <method> <path>" 404, not the writer's 404 for a
/// slot it doesn't have, nor the controller's for a board that isn't its
/// peer.
fn lacks_route(words: &str) -> bool {
    words.contains("answered 404") && words.contains("no route for")
}

/// The slots in HeliOS's update status; `None` on a board without them.
fn slot_set(status: &Value) -> Result<Option<SlotSet>, DriverError> {
    let slots = &status["slots"];
    if slots["layout"].as_str() != Some("slots") {
        return Ok(None);
    }
    let text = |name: &str| slots[name].as_str().unwrap_or_default().to_string();
    let installed = serde_json::from_value::<Vec<Slot>>(slots["installed"].clone())
        .map_err(|error| DriverError::Other(format!("the board's slots: {error}")))?;
    Ok(Some(SlotSet {
        running: text("active"),
        default: text("default"),
        previous: text("previous"),
        trial: text("trial"),
        free_bytes: slots["free_bytes"].as_u64().unwrap_or(0),
        reserve_bytes: slots["reserve_bytes"].as_u64().unwrap_or(0),
        slots: installed,
    }))
}

/// The installed slots' ids in HeliOS's update status.
fn slot_ids(status: &Value) -> Vec<String> {
    status["slots"]["installed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|slot| slot["id"].as_str().map(str::to_string))
        .collect()
}

/// Where HeliOS's task `update_id` (staging an upload) stands: `None`
/// while it runs.
fn staged(status: &Value, update_id: &str) -> Option<Result<(), String>> {
    let task = &status["task"];
    if task["update_id"].as_str() != Some(update_id) {
        return None;
    }
    match task["step"].as_str().unwrap_or_default() {
        "staging" => None,
        "failed" => Some(Err(task["error"]
            .as_str()
            .or(status["last_error"].as_str())
            .unwrap_or("the board's writer failed")
            .to_string())),
        _ => Some(Ok(())),
    }
}

#[async_trait]
impl SlotsCapability for GatewaySlots {
    async fn slots(&self, _device: &Identity) -> Result<Option<SlotSet>, DriverError> {
        slot_set(&self.0.get("update/status").await?)
    }

    async fn command(&self, _device: &Identity, command: &SlotCommand) -> Result<(), DriverError> {
        // Booting another slot is HeliOS's switch (on trial); the rest are
        // the writer's own commands (`update/slots/<command>`).
        let (path, id) = match command {
            SlotCommand::BootOnce { id } => ("update/slots/switch".to_string(), id),
            other => (
                format!("update/slots/{}", other.writer_command()),
                &other.id().to_string(),
            ),
        };
        match self.0.post(&path, json!({ "slot": id })).await {
            Err(DriverError::Other(words)) if lacks_route(&words) => {
                Err(DriverError::Unsupported(format!(
                    "this board's HeliOS can't {} through the robot controller yet",
                    command.describe()
                )))
            }
            result => result.map(|_| ()),
        }
    }

    async fn install(
        &self,
        _device: &Identity,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        if install.pin {
            return Err(DriverError::Unsupported(
                "through the robot controller a slot can't be pinned as it installs".into(),
            ));
        }
        if install.boot && !install.make_default {
            return Err(DriverError::Unsupported(
                "through the robot controller a slot can't boot as a trial only (it would \
                 become the default): pass --make-default"
                    .into(),
            ));
        }
        progress.step_started(UpdateStep::Transfer);
        let sink = progress.clone();
        let image_url =
            atlas_driver::peer_upload::upload(&self.0.peer, &install.bundle, move |done| {
                sink.step_progress(UpdateStep::Transfer, done);
            })
            .await?;
        let before = slot_ids(&self.0.get("update/status").await?);
        progress.step_started(UpdateStep::Apply);
        let answer = match self
            .0
            .post(
                "update/apply",
                json!({
                    "image_url": image_url,
                    "sha256": install.bundle.sha256,
                    "reboot": install.boot,
                }),
            )
            .await
        {
            Ok(answer) => answer,
            // HeliOS answers once the slot is staged, and with `reboot` the
            // restart can end the request first.
            Err(DriverError::Unreachable(_)) if install.boot => {
                return self.restarted_into_new_slot(&before, progress).await;
            }
            Err(error) => return Err(error),
        };
        // (HeliOS's `message` is written for an HTTP caller: "boot it later
        // with POST /v1/update/apply"; Atlas says what happened itself.)
        let update_id = answer["update_id"].as_str().unwrap_or_default().to_string();
        let started = Instant::now();
        loop {
            tokio::time::sleep(STAGE_POLL).await;
            match self.0.get("update/status").await {
                Ok(status) => match staged(&status, &update_id) {
                    Some(Ok(())) if install.boot => {
                        progress.log("installed; the board is restarting into it once");
                        return self.restarted_into_new_slot(&before, progress).await;
                    }
                    Some(Ok(())) => {
                        progress.log("installed");
                        return Ok(());
                    }
                    Some(Err(reason)) => {
                        return Err(DriverError::Other(atlas_driver::refusal(&reason)));
                    }
                    None => {
                        if let Some(percent) = status["progress_percent"].as_u64() {
                            progress.step_progress(UpdateStep::Apply, percent as f32 / 100.0);
                        }
                    }
                },
                // Restarting into the new slot: the board is gone a while.
                Err(_) if install.boot => {
                    return self.restarted_into_new_slot(&before, progress).await;
                }
                Err(error) => return Err(error),
            }
            if started.elapsed() >= STAGE_TIMEOUT {
                return Err(DriverError::Other(format!(
                    "the board was still staging the image after {} min",
                    STAGE_TIMEOUT.as_secs() / 60
                )));
            }
        }
    }
}

impl GatewaySlots {
    /// Waits for the board to go and come back with a slot it didn't have
    /// (`before`): the image staged, and the board restarted into it.
    async fn restarted_into_new_slot(
        &self,
        before: &[String],
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        progress.step_started(UpdateStep::Reboot);
        let started = Instant::now();
        let mut went = false;
        while started.elapsed() < BOOT_TIMEOUT {
            tokio::time::sleep(BOOT_POLL).await;
            // While it restarts the controller doesn't route to it (404).
            let Ok(status) = self.0.get("update/status").await else {
                went = true;
                continue;
            };
            if slot_ids(&status).iter().any(|id| !before.contains(id)) {
                return Ok(());
            }
            if went {
                return Err(DriverError::Other(
                    "the board restarted without the new slot; its writer refused the image".into(),
                ));
            }
        }
        Err(DriverError::Unreachable(format!(
            "the board didn't answer through the controller {} min after its restart",
            BOOT_TIMEOUT.as_secs() / 60
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helios_update_status_gives_the_slots() {
        let status = json!({
            "phase": "idle",
            "slots": {
                "active": "s904", "staged": null, "layout": "slots",
                "default": "s904", "previous": "s890", "trial": null,
                "free_bytes": 900, "reserve_bytes": 100,
                "installed": [{ "id": "s904", "version": "2026.1.0-dev.904", "size": 5 }]
            }
        });
        let set = slot_set(&status).unwrap().unwrap();
        assert_eq!(set.running, "s904");
        assert_eq!(set.previous, "s890");
        assert_eq!(set.trial, "");
        assert_eq!(set.room(), 800);
        assert_eq!(set.slots[0].version, "2026.1.0-dev.904");
        // As Raze #2's HeliOS answered through a SystemCore (2026-11-03).
        let real: Value =
            serde_json::from_str(include_str!("../tests/helios-update-status.json")).unwrap();
        let set = slot_set(&real).unwrap().unwrap();
        assert_eq!(set.slots[0].id, set.running);
        assert_eq!(set.slots[0].version, "2026.2.0-dev.904");
        let guest = json!({ "slots": { "layout": "none", "installed": [] } });
        assert_eq!(slot_set(&guest).unwrap(), None);
    }

    #[test]
    fn only_a_missing_route_is_a_board_too_old_for_the_command() {
        assert!(lacks_route(
            "the board's HeliOS answered 404 Not Found: no route for POST /v1/update/slots/pin"
        ));
        // The writer's own answer for a slot that isn't there passes through.
        assert!(!lacks_route(
            "the board's HeliOS answered 404 Not Found: no installed slot s907"
        ));
        assert!(!lacks_route(
            "the board's HeliOS answered 404 Not Found: raze-6d57 is not a peer"
        ));
        assert!(!lacks_route(
            "the board's HeliOS answered 409 Conflict: s907 is pinned; unpin it first"
        ));
    }

    #[test]
    fn staging_ends_staged_or_with_the_writers_error() {
        let task = |step: &str| json!({ "task": { "update_id": "u7", "step": step, "error": "slots: no room" } });
        assert_eq!(staged(&task("staging"), "u7"), None);
        assert_eq!(staged(&task("staged"), "u7"), Some(Ok(())));
        assert_eq!(staged(&task("applying"), "u7"), Some(Ok(())));
        assert_eq!(
            staged(&task("failed"), "u7"),
            Some(Err("slots: no room".into()))
        );
        // Another task's state says nothing about this one.
        assert_eq!(staged(&task("staged"), "u6"), None);
    }
}
