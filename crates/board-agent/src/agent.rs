//! The actions and the `update.*` keys (Orion `docs/device-agent.md`).
//!
//! `update` starts `update stage-url` in the background and succeeds with
//! `phase = "staging"` once the writer has taken its lock; when the stage
//! ends well the agent runs `update apply`, which restarts into the new slot
//! on trial. Everything after that is told by the writer's state, which the
//! agent publishes as the `update.*` keys: after connecting, whenever
//! `update.json` changes, and every [`Config::republish`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orion_client::ActionReporter;
use orion_control_plane::{
    ActionRequest, TypedConfigValue, action_names, action_status_keys, update_action,
};
use tokio::process::Child;

use crate::config::Config;
use crate::published::{BootRecord, LastEvent, update_status_entries};
pub use crate::stage::StageRequest;
use crate::writer::{StderrTail, Writer, WriterStatus};

/// The node actions the agent claims. Published as `action.claimed` (a
/// comma-separated list), so operators can tell what this node offers.
pub const CLAIMED_ACTIONS: [&str; 11] = [
    action_names::UPDATE,
    action_names::UPDATE_CANCEL,
    action_names::UPDATE_ROLLBACK,
    action_names::REBOOT,
    action_names::LOCATE,
    crate::clock::CLOCK_SET,
    USB_ROUTE_SET,
    SLOT_BOOT_ONCE,
    SLOT_SET_DEFAULT,
    SLOT_REMOVE,
    SLOT_PIN,
];

/// Slots boards (the slots writer): boot an installed slot once (`id`),
/// make one that booted well the default (`id`), remove one (`id`), pin or
/// unpin one (`id`, `pinned`). Installing is `update` (with `apply: false`
/// to install only, `pin`). The slots and the free space are published
/// as `update.slots` (JSON) and `update.free_bytes`.
pub const SLOT_BOOT_ONCE: &str = "slot.boot-once";
pub const SLOT_SET_DEFAULT: &str = "slot.set-default";
pub const SLOT_REMOVE: &str = "slot.remove";
pub const SLOT_PIN: &str = "slot.pin";

/// Turns the USB link's route onto the robot network on or off (`on`, a
/// bool): the package's `usb-route set on|off`, kept across restarts.
pub const USB_ROUTE_SET: &str = "usb-route.set";

/// The status key listing [`CLAIMED_ACTIONS`].
pub const CLAIMED_KEY: &str = "action.claimed";

/// The writer's exit status for "refused, nothing changed".
const WRITER_REFUSED: i32 = 3;

/// How long `update` waits for the writer to take its lock.
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a new update waits for a failed stage's writer to exit.
const FAILED_STAGE_EXIT: Duration = Duration::from_secs(2);

fn phase(value: &str) -> BTreeMap<String, TypedConfigValue> {
    BTreeMap::from([(
        update_action::OUTPUT_PHASE.to_owned(),
        TypedConfigValue::String(value.into()),
    )])
}

/// What was last published: the writer's state, the newest event and the
/// boot record.
type Published = (WriterStatus, Option<LastEvent>, Option<BootRecord>);

/// The agent: one per connection to the node.
pub struct Agent {
    config: Arc<Config>,
    writer: Writer,
    reporter: ActionReporter,
    boot_id: String,
    staging: Mutex<Option<Staging>>,
    generation: AtomicU64,
    /// Held while deciding to apply a finished stage, and by `update.cancel`,
    /// so a cancel never races the restart.
    apply_gate: tokio::sync::Mutex<()>,
    /// What was last published, to publish only changes between ticks.
    published: tokio::sync::Mutex<Option<Published>>,
    /// Whether the last publish failed (its error is logged once a run).
    failing: AtomicBool,
    /// Told when the node refuses the agent's keys: it no longer has the
    /// claims (a restarted orion-node the watch hasn't noticed), so the
    /// agent must claim again (`run` starts over).
    pub(crate) claims_lost: tokio::sync::Notify,
}

impl Agent {
    pub fn new(config: Arc<Config>, reporter: ActionReporter) -> Arc<Self> {
        let boot_id = std::fs::read_to_string(&config.boot_id_file)
            .map(|id| id.trim().to_owned())
            .unwrap_or_default();
        Arc::new(Self {
            writer: Writer::new(&config.writer, &config.run_dir),
            config,
            reporter,
            boot_id,
            staging: Mutex::new(None),
            generation: AtomicU64::new(0),
            apply_gate: tokio::sync::Mutex::new(()),
            published: tokio::sync::Mutex::new(None),
            failing: AtomicBool::new(false),
            claims_lost: tokio::sync::Notify::new(),
        })
    }

    fn staging(&self) -> std::sync::MutexGuard<'_, Option<Staging>> {
        self.staging.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The writer's state: update.json, else `update status`.
    async fn current(&self) -> WriterStatus {
        if let Some(status) = self.writer.read_status() {
            return status;
        }
        match self.writer.status().await {
            Ok(status) => status,
            Err(error) => {
                crate::log(&format!("update status: {error}"));
                WriterStatus {
                    state: update_action::STATE_IDLE.into(),
                    error,
                    ..WriterStatus::default()
                }
            }
        }
    }

    /// Publishes the writer's state when it changed since the last publish
    /// (or always, with `force`). A failed publish (no claim yet after a
    /// reconnect, the node restarting) is retried by the next one.
    pub async fn publish(&self, force: bool) {
        let mut published = self.published.lock().await;
        let now = (
            self.current().await,
            LastEvent::read(&self.config.data_dir.join("events.jsonl")),
            BootRecord::read(&self.config.run_dir, &self.config.data_dir, &self.boot_id),
        );
        if !force && published.as_ref() == Some(&now) {
            return;
        }
        let mut entries = update_status_entries(
            &self.reporter,
            &now.0,
            &self.boot_id,
            now.1.as_ref(),
            now.2.as_ref(),
        );
        crate::published::keep_fitting(&mut entries, !self.failing.load(Ordering::Relaxed));
        match self.reporter.publish_status(entries).await {
            Ok(()) => {
                *published = Some(now);
                self.failing.store(false, Ordering::Relaxed);
            }
            Err(error) => {
                // Logged once per run of failures, then retried every tick.
                let error = error.to_string();
                if !self.failing.swap(true, Ordering::Relaxed) {
                    crate::log(&format!("publishing the update state: {error}"));
                }
                if crate::published::refused_for_claims(&error) {
                    self.claims_lost.notify_one();
                }
                *published = None;
            }
        }
    }

    /// Runs `update status` once (it reconciles a stage left over from an
    /// earlier boot), then publishes.
    pub async fn publish_initial(&self) {
        if let Err(error) = self.writer.status().await {
            crate::log(&format!("update status: {error}"));
        }
        self.publish(true).await;
    }

    /// Follows update.json and republishes periodically, until dropped.
    pub async fn follow(self: Arc<Self>) {
        let mut tick = tokio::time::interval(self.config.poll);
        let mut last_full = tokio::time::Instant::now();
        loop {
            tick.tick().await;
            let force = last_full.elapsed() >= self.config.republish;
            if force {
                last_full = tokio::time::Instant::now();
            }
            self.publish(force).await;
        }
    }

    /// Mirrors a final action state into `action.<id>.*`.
    async fn publish_action(&self, action_id: &str, state: &str, error: Option<&str>) {
        let mut entries = vec![self.reporter.node_status_entry(
            action_status_keys::key(action_id, action_status_keys::STATE),
            TypedConfigValue::String(state.into()),
        )];
        if let Some(error) = error {
            entries.push(self.reporter.node_status_entry(
                action_status_keys::key(action_id, action_status_keys::ERROR),
                TypedConfigValue::String(error.into()),
            ));
        }
        let _ = self.reporter.publish_status(entries).await;
    }

    async fn reject(&self, action_id: &str, reason: String) {
        crate::log(&format!("{action_id}: rejected: {reason}"));
        self.publish_action(action_id, "rejected", Some(&reason))
            .await;
        let _ = self.reporter.reject(action_id, reason).await;
    }

    async fn fail(&self, action_id: &str, reason: String) {
        crate::log(&format!("{action_id}: failed: {reason}"));
        self.publish_action(action_id, "failed", Some(&reason))
            .await;
        let _ = self.reporter.fail(action_id, reason).await;
    }

    async fn succeed(&self, action_id: &str, output: BTreeMap<String, TypedConfigValue>) {
        self.publish_action(action_id, "succeeded", None).await;
        let _ = self.reporter.succeed(action_id, output).await;
    }

    /// Runs one claimed action and reports its outcome.
    pub async fn handle(self: &Arc<Self>, request: ActionRequest) {
        let id = request.action_id.clone();
        crate::log(&format!("{id}: {}", request.name));
        let uint = |key: &str, default: u64| match request.args.get(key) {
            Some(TypedConfigValue::UInt(value)) => *value,
            _ => default,
        };
        match request.name.as_str() {
            action_names::UPDATE => self.start_update(&id, &request.args).await,
            action_names::UPDATE_CANCEL => self.cancel_update(&id).await,
            action_names::UPDATE_ROLLBACK => self.rollback(&id).await,
            action_names::REBOOT => {
                // Report first: the reboot ends this process and the node's
                // action record.
                self.succeed(&id, phase(update_action::PHASE_REBOOTING))
                    .await;
                self.event("reboot", "restart requested through Orion", &[])
                    .await;
                self.reboot(uint("delay_ms", 0)).await;
            }
            action_names::LOCATE => {
                let enabled = !matches!(
                    request.args.get("enabled"),
                    Some(TypedConfigValue::Bool(false))
                );
                match self.locate(enabled, uint("duration_ms", 10_000)).await {
                    Ok(()) => self.succeed(&id, BTreeMap::new()).await,
                    Err(error) => self.fail(&id, error).await,
                }
            }
            crate::clock::CLOCK_SET => self.set_clock(&id, &request.args).await,
            USB_ROUTE_SET => {
                let Some(TypedConfigValue::Bool(on)) = request.args.get("on") else {
                    return self.reject(&id, "`on` (bool) is required".into()).await;
                };
                let mut command = self.config.usb_route_command.clone();
                command.extend(["set".to_owned(), if *on { "on" } else { "off" }.to_owned()]);
                match run_command(&command).await {
                    Ok(()) => {
                        let output =
                            BTreeMap::from([("on".to_owned(), TypedConfigValue::Bool(*on))]);
                        self.succeed(&id, output).await;
                    }
                    Err(error) => self.fail(&id, format!("usb-route: {error}")).await,
                }
            }
            SLOT_BOOT_ONCE | SLOT_SET_DEFAULT | SLOT_REMOVE | SLOT_PIN => {
                self.slot_action(&id, &request).await;
            }
            other => {
                self.reject(&id, format!("unsupported action `{other}`"))
                    .await;
            }
        }
    }

    /// `update.rollback`: switch to the previous confirmed slot, report,
    /// then reboot.
    async fn rollback(&self, id: &str) {
        let staging = self.staging().is_some()
            || self
                .writer
                .read_status()
                .is_some_and(|s| s.state == "staging");
        if staging {
            return self
                .reject(
                    id,
                    "an update is staging; cancel it with `update.cancel` first".into(),
                )
                .await;
        }
        if let Err(reason) = self.writer.run(&["rollback", "--no-reboot"]).await {
            return self.reject(id, reason).await;
        }
        self.publish(false).await;
        // Report first: the reboot ends this process and the node's action
        // record.
        self.succeed(id, phase(update_action::PHASE_REBOOTING))
            .await;
        self.reboot(0).await;
    }

    /// Appends to the board's event log; best effort (an image without the
    /// package's `event` loses only the line).
    async fn event(&self, kind: &str, message: &str, data: &[String]) {
        let mut command = self.config.event_command.clone();
        command.extend([kind.to_owned(), message.to_owned()]);
        command.extend(data.iter().cloned());
        if let Err(error) = run_command(&command).await {
            crate::log(&format!("event {kind}: {error}"));
        }
    }

    async fn reboot(&self, delay_ms: u64) {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if let Err(error) = run_command(&self.config.reboot_command).await {
            crate::log(&format!("reboot: {error}"));
        }
    }

    /// Locate goes through the package's request file, which
    /// board-locate.path turns into board-locate.service (the LED ring's
    /// locate pattern); stopping it stops that service.
    /// `clock.set`: validates the time (clock.rs), sets it and logs it.
    async fn set_clock(&self, id: &str, args: &BTreeMap<String, TypedConfigValue>) {
        let secs = match crate::clock::requested_time(args) {
            Ok(secs) => secs,
            Err(reason) => return self.reject(id, reason).await,
        };
        let old = crate::clock::now();
        let mut command = self.config.set_clock_command.clone();
        command.push(format!("@{secs}"));
        if let Err(error) = run_command(&command).await {
            return self
                .fail(id, format!("setting the clock failed: {error}"))
                .await;
        }
        let new = crate::clock::now();
        self.event(
            "clock.set",
            "clock set through Orion",
            &[format!("old={old}"), format!("new={new}")],
        )
        .await;
        let output = BTreeMap::from([
            ("old".to_owned(), TypedConfigValue::Int(old)),
            ("new".to_owned(), TypedConfigValue::Int(new)),
        ]);
        self.succeed(id, output).await;
    }

    async fn locate(&self, enabled: bool, duration_ms: u64) -> Result<(), String> {
        if !enabled {
            return run_command(&self.config.locate_stop_command).await;
        }
        let requests = self.writer.run_dir().join("requests");
        let seconds = duration_ms.div_ceil(1000).max(1);
        tokio::fs::write(requests.join("locate"), format!("{seconds}\n"))
            .await
            .map_err(|error| format!("can't request locate in {}: {error}", requests.display()))
    }
}

/// Runs a configured command (program and arguments), as Orion's for the
/// board's event log.
async fn run_command(command: &[String]) -> Result<(), String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| "no command configured".to_owned())?;
    let output = tokio::process::Command::new(program)
        .args(args)
        .env("BOARD_EVENT_SOURCE", crate::EVENT_SOURCE)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|error| format!("can't run {program}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr
            .lines()
            .last()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} failed ({})", command.join(" "), output.status)))
    }
}

mod slots;
mod update;

use update::Staging;
#[cfg(test)]
mod tests;
