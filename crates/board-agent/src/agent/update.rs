//! The `update` and `update.cancel` actions: a stage run in the background.

use super::*;

/// A stage running in the background.
pub(super) struct Staging {
    pub(super) generation: u64,
    pub(super) sha256: String,
    pub(super) flags: Arc<StageFlags>,
    pub(super) task: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
pub(super) struct StageFlags {
    pub(super) cancelled: AtomicBool,
    /// The writer exited; the agent may still be reading its last stderr
    /// line or applying, but a retry starts a new stage.
    pub(super) exited: AtomicBool,
}

impl Agent {
    /// `update`: starts the stage and succeeds with `phase = "staging"` once
    /// the writer runs it.
    pub(super) async fn start_update(
        self: &Arc<Self>,
        id: &str,
        args: &BTreeMap<String, TypedConfigValue>,
    ) {
        let request = match StageRequest::from_args(args) {
            Ok(request) => request,
            Err(reason) => return self.reject(id, reason).await,
        };
        self.await_failed_stage().await;
        let running = self
            .staging()
            .as_ref()
            .filter(|s| !s.flags.exited.load(Ordering::SeqCst))
            .map(|s| s.sha256.clone());
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
        let (child, tail) = match self.writer.spawn_stage(
            &request.image_url,
            &request.sha256,
            request.size,
            request.pin,
        ) {
            Ok(spawned) => spawned,
            Err(error) => return self.fail(id, error).await,
        };
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let flags = Arc::new(StageFlags::default());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(self.clone().run_stage(
            generation,
            child,
            tail,
            started_tx,
            flags.clone(),
            request.apply,
        ));
        *self.staging() = Some(Staging {
            generation,
            sha256: request.sha256,
            flags,
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

    /// A stage whose writer already recorded its end (not `staging` any
    /// more) but hasn't exited yet: waits a moment for it, so an update
    /// sent right after the failure starts anew, not "already started".
    async fn await_failed_stage(&self) {
        let deadline = tokio::time::Instant::now() + FAILED_STAGE_EXIT;
        loop {
            let Some(flags) = self.staging().as_ref().map(|s| s.flags.clone()) else {
                return;
            };
            let ended = self
                .writer
                .read_status()
                .is_some_and(|s| s.state != "staging");
            if flags.exited.load(Ordering::SeqCst)
                || !ended
                || tokio::time::Instant::now() >= deadline
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The background stage: waits for the writer, then applies.
    async fn run_stage(
        self: Arc<Self>,
        generation: u64,
        mut child: Child,
        tail: StderrTail,
        started: tokio::sync::oneshot::Sender<Result<(), String>>,
        flags: Arc<StageFlags>,
        apply: bool,
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
        flags.exited.store(true, Ordering::SeqCst);
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
        if flags.cancelled.load(Ordering::SeqCst) {
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
        // restart was refused). An install only (a slots board) stops at
        // `staged`.
        self.publish(false).await;
        if !apply {
            return;
        }
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
    pub(super) async fn cancel_update(&self, id: &str) {
        let running = self.staging().take();
        if let Some(running) = &running {
            running.flags.cancelled.store(true, Ordering::SeqCst);
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
}
