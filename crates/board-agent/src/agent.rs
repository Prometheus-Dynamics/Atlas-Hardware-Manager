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
    ActionRequest, StatusEntry, TypedConfigValue, action_names, action_status_keys, update_action,
};
use tokio::process::Child;

use crate::config::Config;
pub use crate::stage::StageRequest;
use crate::writer::{StderrTail, Writer, WriterStatus};

/// The node actions the agent claims. Published as `action.claimed` (a
/// comma-separated list), so operators can tell what this node offers.
pub const CLAIMED_ACTIONS: [&str; 6] = [
    action_names::UPDATE,
    action_names::UPDATE_CANCEL,
    action_names::UPDATE_ROLLBACK,
    action_names::REBOOT,
    action_names::LOCATE,
    crate::clock::CLOCK_SET,
];

/// The status key listing [`CLAIMED_ACTIONS`].
pub const CLAIMED_KEY: &str = "action.claimed";

/// The writer's exit status for "refused, nothing changed".
const WRITER_REFUSED: i32 = 3;

/// How long `update` waits for the writer to take its lock.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// The `update.*` entries for `status` under `node/<id>` (TTL 0: the node's
/// maximum).
pub fn update_status_entries(
    reporter: &ActionReporter,
    status: &WriterStatus,
    boot_id: &str,
) -> Vec<StatusEntry> {
    let text = |value: &str| TypedConfigValue::String(value.to_owned());
    let state = if status.state.is_empty() {
        update_action::STATE_IDLE
    } else {
        status.state.as_str()
    };
    vec![
        reporter.node_status_entry(update_action::KEY_STATE, text(state)),
        reporter.node_status_entry(update_action::KEY_SLOT_ACTIVE, text(&status.slot_active)),
        reporter.node_status_entry(update_action::KEY_SLOT_STAGED, text(&status.slot_staged)),
        reporter.node_status_entry(
            update_action::KEY_VERSION_ACTIVE,
            text(&status.version_active),
        ),
        reporter.node_status_entry(
            update_action::KEY_VERSION_STAGED,
            text(&status.version_staged),
        ),
        reporter.node_status_entry(
            update_action::KEY_PROGRESS,
            TypedConfigValue::UInt(status.progress.min(1000)),
        ),
        reporter.node_status_entry(update_action::KEY_ERROR, text(&status.error)),
        reporter.node_status_entry(update_action::KEY_BOOT_ID, text(boot_id)),
        reporter.node_status_entry(CLAIMED_KEY, text(&CLAIMED_ACTIONS.join(","))),
    ]
}

fn phase(value: &str) -> BTreeMap<String, TypedConfigValue> {
    BTreeMap::from([(
        update_action::OUTPUT_PHASE.to_owned(),
        TypedConfigValue::String(value.into()),
    )])
}

/// A stage running in the background.
struct Staging {
    generation: u64,
    sha256: String,
    cancelled: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

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
    /// The status last published, to publish only changes between ticks.
    published: tokio::sync::Mutex<Option<WriterStatus>>,
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
        let status = self.current().await;
        if !force && published.as_ref() == Some(&status) {
            return;
        }
        let entries = update_status_entries(&self.reporter, &status, &self.boot_id);
        match self.reporter.publish_status(entries).await {
            Ok(()) => *published = Some(status),
            Err(_) => *published = None,
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
            other => {
                self.reject(&id, format!("unsupported action `{other}`"))
                    .await;
            }
        }
    }

    /// `update`: starts the stage and succeeds with `phase = "staging"` once
    /// the writer runs it.
    async fn start_update(self: &Arc<Self>, id: &str, args: &BTreeMap<String, TypedConfigValue>) {
        let request = match StageRequest::from_args(args) {
            Ok(request) => request,
            Err(reason) => return self.reject(id, reason).await,
        };
        let running = self.staging().as_ref().map(|s| s.sha256.clone());
        match running {
            // A retry of the running update: already started.
            Some(sha256) if sha256 == request.sha256 => {
                return self.succeed(id, phase(update_action::PHASE_STAGING)).await;
            }
            Some(_) => {
                return self
                    .reject(
                        id,
                        "another update is staging; cancel it with `update.cancel` first".into(),
                    )
                    .await;
            }
            None => {}
        }
        if self
            .writer
            .read_status()
            .is_some_and(|s| s.state == "staging")
            && self
                .writer
                .stage_pid()
                .is_some_and(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
        {
            return self
                .reject(
                    id,
                    "a stage started outside the agent is running; cancel it with `update.cancel` first"
                        .into(),
                )
                .await;
        }
        let (child, tail) =
            match self
                .writer
                .spawn_stage(&request.image_url, &request.sha256, request.size)
            {
                Ok(spawned) => spawned,
                Err(error) => return self.fail(id, error).await,
            };
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(self.clone().run_stage(
            generation,
            child,
            tail,
            started_tx,
            cancelled.clone(),
        ));
        *self.staging() = Some(Staging {
            generation,
            sha256: request.sha256,
            cancelled,
            task,
        });
        match tokio::time::timeout(START_TIMEOUT, started_rx).await {
            Ok(Ok(Err(reason))) => self.reject(id, reason).await,
            // Started, or still starting after the timeout: the writer's
            // state tells the rest.
            _ => {
                self.publish(false).await;
                self.succeed(id, phase(update_action::PHASE_STAGING)).await;
            }
        }
    }

    /// The background stage: waits for the writer, then applies.
    async fn run_stage(
        self: Arc<Self>,
        generation: u64,
        mut child: Child,
        tail: StderrTail,
        started: tokio::sync::oneshot::Sender<Result<(), String>>,
        cancelled: Arc<AtomicBool>,
    ) {
        let pid = child.id();
        let mut started = Some(started);
        let exit = loop {
            tokio::select! {
                exit = child.wait() => break exit,
                () = tokio::time::sleep(Duration::from_millis(50)), if started.is_some() => {
                    if pid.is_some() && self.writer.stage_pid() == pid
                        && let Some(started) = started.take()
                    {
                        let _ = started.send(Ok(()));
                    }
                }
            }
        };
        let ok = exit.as_ref().is_ok_and(|status| status.success());
        tail.finished(Duration::from_secs(1)).await;
        let reason = || {
            tail.reason().unwrap_or_else(|| match &exit {
                Ok(status) => format!("the writer failed ({status})"),
                Err(error) => format!("the writer failed: {error}"),
            })
        };
        if let Some(started) = started.take() {
            // It ended before we saw it take the lock. A refusal (trial
            // running, another writer; exit status 3) is the action's
            // answer; any other failure happened while staging and is in
            // the writer's state.
            let refused = exit
                .as_ref()
                .is_ok_and(|status| status.code() == Some(WRITER_REFUSED));
            if refused {
                let _ = started.send(Err(reason()));
                self.clear_staging(generation);
                self.publish(false).await;
                return;
            }
            let _ = started.send(Ok(()));
        }
        self.clear_staging(generation);
        let _gate = self.apply_gate.lock().await;
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        if !ok {
            crate::log(&format!("staging failed: {}", reason()));
            // Turns a stage that died without recording why into an error.
            let _ = self.writer.status().await;
            self.publish(false).await;
            return;
        }
        // `staged`, then `rebooting` (or `staged` with the reason when the
        // restart was refused).
        self.publish(false).await;
        if let Err(error) = self.writer.run(&["apply"]).await {
            crate::log(&format!("apply: {error}"));
        }
        self.publish(false).await;
    }

    fn clear_staging(&self, generation: u64) {
        let mut staging = self.staging();
        if staging.as_ref().is_some_and(|s| s.generation == generation) {
            *staging = None;
        }
    }

    /// `update.cancel`: stops a running stage, or forgets a staged update.
    async fn cancel_update(&self, id: &str) {
        let running = self.staging().take();
        if let Some(running) = &running {
            running.cancelled.store(true, Ordering::SeqCst);
        }
        let result = {
            let _gate = self.apply_gate.lock().await;
            self.writer.run(&["cancel"]).await
        };
        if let Some(running) = running {
            let _ = running.task.await;
        }
        self.publish(false).await;
        match result {
            Ok(out) => {
                let value = if out.trim() == "idle" {
                    update_action::PHASE_IDLE
                } else {
                    update_action::PHASE_CANCELLED
                };
                self.succeed(id, phase(value)).await;
            }
            Err(error) => self.fail(id, error).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, TypedConfigValue)]) -> BTreeMap<String, TypedConfigValue> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn update_arguments_are_checked() {
        let text = |value: &str| TypedConfigValue::String(value.into());
        let good = args(&[
            ("image_url", text("http://10.0.0.5:7700/images/t/x.img.xz")),
            ("sha256", text(&"AB".repeat(32))),
            ("size", TypedConfigValue::UInt(42)),
        ]);
        let request = StageRequest::from_args(&good).unwrap();
        assert_eq!(request.sha256, "ab".repeat(32));
        assert_eq!(request.size, 42);

        let mut bad = good.clone();
        bad.insert("image_url".into(), text("file:///etc/shadow"));
        assert!(StageRequest::from_args(&bad).unwrap_err().contains("http"));
        let mut bad = good.clone();
        bad.insert("sha256".into(), text("abc"));
        assert!(
            StageRequest::from_args(&bad)
                .unwrap_err()
                .contains("sha256")
        );
        let mut bad = good;
        bad.remove("size");
        assert!(StageRequest::from_args(&bad).unwrap_err().contains("size"));
    }
}
