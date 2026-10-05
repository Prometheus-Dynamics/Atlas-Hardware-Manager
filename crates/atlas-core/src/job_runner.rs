use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;

use atlas_driver::{
    Concurrency, DeviceKey, DriverError, Family, ProgressSink, ProgressUpdate, ReleaseRef,
    UpdateOutcome,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio_util::sync::CancellationToken;

use crate::atlas::Inner;
use crate::inventory::Upsert;
use crate::jobs::{
    DeviceJobState, DeviceJobStatus, JobId, JobPlan, JobRecord, JobState, JobSummary,
    PlannedDevice, StagedRollout, UpdateRequest,
};
use crate::time::now_ms;
use crate::{CoreError, Event};

pub(crate) fn plan(inner: &Inner, request: &UpdateRequest) -> Result<JobPlan, CoreError> {
    let mut seen = BTreeSet::new();
    let keys: Vec<&DeviceKey> = request
        .devices
        .iter()
        .filter(|key| seen.insert((*key).clone()))
        .collect();
    if keys.is_empty() {
        return Err(CoreError::EmptySelection);
    }

    let mut devices = Vec::with_capacity(keys.len());
    {
        let state = inner.state();
        for key in keys {
            let record = state
                .inventory
                .get(key)
                .ok_or_else(|| CoreError::UnknownDevice(key.clone()))?;
            let live = state
                .live
                .get(key)
                .ok_or_else(|| CoreError::DeviceOffline(key.clone()))?;
            let update = live
                .capabilities
                .update
                .as_ref()
                .ok_or_else(|| CoreError::NoUpdateCapability(key.clone()))?;
            let target = request
                .releases
                .get(&key.family)
                .ok_or_else(|| CoreError::NoReleaseForFamily(key.family.clone()))?;
            let release = ReleaseRef {
                family: key.family.clone(),
                version: target.version.clone(),
                artifact: target.artifact.clone(),
            };
            let plan = update.plan(&record.identity, &release)?;
            devices.push(PlannedDevice {
                device: key.clone(),
                name: record.display_name(),
                from_version: record.identity.primary_version().map(str::to_string),
                release,
                plan,
                canary: false,
            });
        }
    }

    let mut per_family: BTreeMap<Family, usize> = BTreeMap::new();
    for device in &devices {
        *per_family.entry(device.device.family.clone()).or_default() += 1;
    }
    let mut canary_chosen = BTreeSet::new();
    for device in &mut devices {
        let count = per_family[&device.device.family];
        let staged = match request.staged {
            StagedRollout::On => count > 1,
            StagedRollout::Off => false,
            StagedRollout::Auto => count >= inner.options.staged_rollout_threshold.max(2),
        };
        if staged && canary_chosen.insert(device.device.family.clone()) {
            device.canary = true;
        }
    }
    Ok(JobPlan { devices })
}

pub(crate) fn start(inner: &Arc<Inner>, request: UpdateRequest) -> Result<JobId, CoreError> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| CoreError::NoRuntime)?;
    let job_plan = plan(inner, &request)?;
    let cancel = CancellationToken::new();
    let (done_tx, done_rx) = watch::channel(false);

    let id = {
        let mut state = inner.state();
        // One job per device at a time: a second would fight the first for
        // the same USB port or disk. Checked under the lock so two clicks
        // cannot both get through.
        if let Some(busy) = job_plan.devices.iter().find(|planned| {
            state.jobs.values().any(|record| {
                record.state == JobState::Running
                    && record.devices.iter().any(|device| {
                        device.device == planned.device && !device.status.is_finished()
                    })
            })
        }) {
            return Err(CoreError::DeviceBusy(busy.device.clone()));
        }
        let id = JobId(state.next_job);
        state.next_job += 1;
        let devices = job_plan
            .devices
            .iter()
            .map(|planned| DeviceJobState {
                device: planned.device.clone(),
                name: planned.name.clone(),
                release: planned.release.clone(),
                plan: planned.plan.clone(),
                canary: planned.canary,
                status: DeviceJobStatus::Queued,
                step: None,
                fraction: 0.0,
                log: Vec::new(),
                started_ms: None,
                finished_ms: None,
            })
            .collect();
        state.jobs.insert(
            id,
            JobRecord {
                id,
                state: JobState::Running,
                created_ms: now_ms(),
                finished_ms: None,
                devices,
                summary: None,
            },
        );
        state.job_done.insert(id, done_rx);
        state.job_cancel.insert(id, cancel.clone());
        id
    };

    inner.events.emit(Event::JobStarted {
        job: id,
        devices: job_plan.devices.iter().map(|d| d.device.clone()).collect(),
    });
    runtime.spawn(run_job(inner.clone(), id, job_plan, cancel, done_tx));
    Ok(id)
}

async fn run_job(
    inner: Arc<Inner>,
    job: JobId,
    job_plan: JobPlan,
    cancel: CancellationToken,
    done: watch::Sender<bool>,
) {
    let mut families: BTreeMap<Family, Vec<PlannedDevice>> = BTreeMap::new();
    for planned in job_plan.devices {
        families
            .entry(planned.device.family.clone())
            .or_default()
            .push(planned);
    }
    futures::future::join_all(
        families
            .into_values()
            .map(|devices| run_family(inner.clone(), job, devices, cancel.clone())),
    )
    .await;

    let (summary, final_state, touches_robots) = {
        let mut state = inner.state();
        let touches_robots = !state.robots.is_empty();
        let Some(record) = state.jobs.get_mut(&job) else {
            return;
        };
        let summary = JobSummary::from_devices(&record.devices);
        record.summary = Some(summary);
        record.finished_ms = Some(now_ms());
        record.state = if cancel.is_cancelled() {
            JobState::Cancelled
        } else {
            JobState::Finished
        };
        let final_state = record.state;
        state.job_cancel.remove(&job);
        (summary, final_state, touches_robots)
    };
    inner.events.emit(Event::JobFinished {
        job,
        state: final_state,
        summary,
    });
    if touches_robots {
        inner.events.emit(Event::RobotsChanged);
    }
    inner.persist();
    let _ = done.send(true);
}

async fn run_family(
    inner: Arc<Inner>,
    job: JobId,
    devices: Vec<PlannedDevice>,
    cancel: CancellationToken,
) {
    let mut devices = devices.into_iter();
    let mut rest: Vec<PlannedDevice> = Vec::new();
    if let Some(first) = devices.next() {
        if first.canary {
            let canary_name = first.name.clone();
            let outcome = run_device(inner.clone(), job, first, cancel.clone()).await;
            if !outcome.is_verified() {
                let reason = format!("staged rollout stopped: {canary_name} did not verify");
                for skipped in devices {
                    finish_device(
                        &inner,
                        job,
                        &skipped.device,
                        DeviceJobStatus::Skipped {
                            reason: reason.clone(),
                        },
                    );
                }
                return;
            }
        } else {
            rest.push(first);
        }
    }
    rest.extend(devices);
    futures::future::join_all(
        rest.into_iter()
            .map(|planned| run_device(inner.clone(), job, planned, cancel.clone())),
    )
    .await;
}

/// Waits for a permit unless the job is cancelled first.
async fn acquire(slot: Arc<Semaphore>, cancel: &CancellationToken) -> Option<OwnedSemaphorePermit> {
    tokio::select! {
        permit = slot.acquire_owned() => permit.ok(),
        () = cancel.cancelled() => None,
    }
}

async fn run_device(
    inner: Arc<Inner>,
    job: JobId,
    planned: PlannedDevice,
    cancel: CancellationToken,
) -> DeviceJobStatus {
    let key = planned.device.clone();
    let Some(_parallel) = acquire(inner.parallel.clone(), &cancel).await else {
        return finish_device(&inner, job, &key, DeviceJobStatus::Cancelled);
    };
    let _exclusive = match &planned.plan.concurrency {
        Concurrency::Parallel => None,
        Concurrency::Exclusive(resource) => {
            match acquire(inner.exclusive_slot(resource), &cancel).await {
                Some(permit) => Some(permit),
                None => return finish_device(&inner, job, &key, DeviceJobStatus::Cancelled),
            }
        }
    };
    if cancel.is_cancelled() {
        return finish_device(&inner, job, &key, DeviceJobStatus::Cancelled);
    }

    // Fetch handles now: the device may have gone offline while queued.
    let handles = {
        let state = inner.state();
        state.live.get(&key).cloned().zip(
            state
                .inventory
                .get(&key)
                .map(|record| (record.identity.clone(), record.link_kind.clone())),
        )
    };
    let Some((live, (identity, link_kind))) = handles else {
        let error = "the device went offline before its turn".to_string();
        return finish_device(&inner, job, &key, DeviceJobStatus::Failed { error });
    };
    let Some(update) = live.capabilities.update.clone() else {
        let error = "the device no longer supports updates".to_string();
        return finish_device(&inner, job, &key, DeviceJobStatus::Failed { error });
    };

    with_device(&inner, job, &key, |device| {
        device.status = DeviceJobStatus::Running;
        device.started_ms = Some(now_ms());
    });
    inner.events.emit(Event::JobDevice {
        job,
        device: key.clone(),
        status: DeviceJobStatus::Running,
    });

    let sink = {
        let inner = inner.clone();
        let key = key.clone();
        ProgressSink::new(move |update| report_progress(&inner, job, &key, update))
    };
    // A driver that panics must fail its device, not strand the job as
    // running forever with its resources held.
    let result =
        AssertUnwindSafe(update.run(&identity, &planned.release, &sink, &cancel.child_token()))
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| {
                let detail = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("no details");
                Err(DriverError::Other(format!("the driver crashed: {detail}")))
            });
    let status = match result {
        Ok(UpdateOutcome::Verified { version }) => DeviceJobStatus::Verified { version },
        Ok(UpdateOutcome::RolledBack { reason }) => DeviceJobStatus::RolledBack { reason },
        Ok(UpdateOutcome::NeedsRecovery { reason }) => DeviceJobStatus::NeedsRecovery { reason },
        Err(DriverError::Cancelled) => DeviceJobStatus::Cancelled,
        Err(error) => DeviceJobStatus::Failed {
            error: error.to_string(),
        },
    };
    if status.is_verified() {
        refresh_identity(&inner, job, &key, &live, link_kind).await;
    }
    finish_device(&inner, job, &key, status)
}

/// Re-reads a device after a verified update so the inventory shows the new version.
async fn refresh_identity(
    inner: &Inner,
    job: JobId,
    key: &DeviceKey,
    live: &crate::atlas::LiveDevice,
    link_kind: atlas_driver::LinkKind,
) {
    let identify = live.driver.identify(&live.candidate);
    match tokio::time::timeout(inner.options.identify_timeout, identify).await {
        Ok(Ok(identity)) => {
            let capabilities = live.driver.capabilities(&identity);
            let kinds = capabilities.kinds_for(identity.mode);
            let (outcome, record) = {
                let mut state = inner.state();
                if let Some(entry) = state.live.get_mut(key) {
                    entry.capabilities = capabilities;
                }
                state.inventory.upsert(identity, link_kind, kinds, now_ms())
            };
            if outcome != Upsert::Unchanged {
                inner.events.emit(Event::DeviceSeen {
                    record: Box::new(record),
                    new: outcome == Upsert::New,
                });
            }
        }
        Ok(Err(error)) => log_line(
            inner,
            job,
            key,
            format!("could not re-read device: {error}"),
        ),
        Err(_) => log_line(
            inner,
            job,
            key,
            "device did not answer after the update".into(),
        ),
    }
}

fn report_progress(inner: &Inner, job: JobId, key: &DeviceKey, update: ProgressUpdate) {
    match update {
        ProgressUpdate::StepStarted { step } => {
            with_device(inner, job, key, |device| {
                device.step = Some(step);
                device.fraction = 0.0;
            });
            inner.events.emit(Event::JobStep {
                job,
                device: key.clone(),
                step,
            });
        }
        ProgressUpdate::StepProgress { step, fraction } => {
            with_device(inner, job, key, |device| {
                device.step = Some(step);
                device.fraction = fraction;
            });
            inner.events.emit(Event::JobProgress {
                job,
                device: key.clone(),
                step,
                fraction,
            });
        }
        ProgressUpdate::Log { message } => log_line(inner, job, key, message),
    }
}

fn log_line(inner: &Inner, job: JobId, key: &DeviceKey, message: String) {
    with_device(inner, job, key, |device| device.push_log(message.clone()));
    inner.events.emit(Event::JobLog {
        job,
        device: key.clone(),
        message,
    });
}

fn with_device(
    inner: &Inner,
    job: JobId,
    key: &DeviceKey,
    change: impl FnOnce(&mut DeviceJobState),
) {
    let mut state = inner.state();
    if let Some(device) = state
        .jobs
        .get_mut(&job)
        .and_then(|record| record.device_mut(key))
    {
        change(device);
    }
}

fn finish_device(
    inner: &Inner,
    job: JobId,
    key: &DeviceKey,
    status: DeviceJobStatus,
) -> DeviceJobStatus {
    with_device(inner, job, key, |device| {
        device.status = status.clone();
        device.finished_ms = Some(now_ms());
    });
    inner.events.emit(Event::JobDevice {
        job,
        device: key.clone(),
        status: status.clone(),
    });
    let record = inner.state().inventory.get(key).cloned();
    if let Some(entry) = record.and_then(|record| crate::activity::for_update(&record, &status)) {
        inner.record_activity(vec![entry]);
    }
    status
}
