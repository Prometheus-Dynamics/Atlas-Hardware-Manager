use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;

use atlas_core::{
    Atlas, DeviceJobStatus, DeviceRecord, Event, JobPlan, Presence, StagedRollout, UpdateRequest,
};
use atlas_driver::{CapabilityKind, Concurrency, DeviceKey, DeviceMode, Family};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::output;

type CommandResult = Result<ExitCode, String>;

pub(crate) async fn ls(atlas: &Atlas, as_json: bool) -> CommandResult {
    let report = atlas.scan().await;
    let devices = atlas.devices();
    if as_json {
        output::json(&json!({ "devices": devices, "scan": report }));
        return Ok(ExitCode::SUCCESS);
    }

    let rows: Vec<Vec<String>> = devices.iter().map(device_row).collect();
    output::table(
        &[
            "NAME", "DEVICE", "MODEL", "VERSION", "MODE", "LINK", "STATE",
        ],
        &rows,
    );
    for warning in &report.warnings {
        output::line(&format!("warning: {warning}"));
    }
    let offline = devices.len() - report.online.len().min(devices.len());
    output::line(&format!(
        "{} online, {offline} offline (scan took {} ms)",
        report.online.len(),
        report.duration_ms
    ));
    Ok(ExitCode::SUCCESS)
}

fn device_row(device: &DeviceRecord) -> Vec<String> {
    vec![
        device.display_name(),
        device.key.to_string(),
        device.identity.model.clone(),
        device.identity.primary_version().unwrap_or("-").to_string(),
        match device.identity.mode {
            DeviceMode::Normal => "normal".into(),
            DeviceMode::Recovery => "recovery".into(),
        },
        device.link_kind.label().to_string(),
        match device.presence {
            Presence::Online => "online".into(),
            Presence::Offline => "offline".into(),
        },
    ]
}

/// Finds one device by name, `family:serial`, or serial.
fn resolve(atlas: &Atlas, selector: &str) -> Result<DeviceKey, String> {
    let devices = atlas.devices();
    if let Some(key) = DeviceKey::parse(selector)
        && devices.iter().any(|device| device.key == key)
    {
        return Ok(key);
    }
    let matches: Vec<&DeviceRecord> = devices
        .iter()
        .filter(|device| {
            device.display_name().eq_ignore_ascii_case(selector)
                || device.key.serial.0.eq_ignore_ascii_case(selector)
        })
        .collect();
    match matches.as_slice() {
        [device] => Ok(device.key.clone()),
        [] => Err(format!(
            "no device matches `{selector}`; run `atlas ls` to see devices"
        )),
        several => Err(format!(
            "`{selector}` matches {} devices ({}); use `family:serial`",
            several.len(),
            several
                .iter()
                .map(|device| device.key.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub(crate) struct UpdateOptions {
    pub(crate) selectors: Vec<String>,
    pub(crate) all: bool,
    pub(crate) version: Option<String>,
    pub(crate) releases: Vec<String>,
    pub(crate) staged: StagedRollout,
    pub(crate) dry_run: bool,
    pub(crate) json: bool,
}

fn build_request(atlas: &Atlas, options: &UpdateOptions) -> Result<UpdateRequest, String> {
    let devices: Vec<DeviceKey> = if options.all {
        atlas
            .devices()
            .into_iter()
            .filter(|device| {
                device.presence == Presence::Online
                    && device.capabilities.contains(&CapabilityKind::Update)
            })
            .map(|device| device.key)
            .collect()
    } else {
        options
            .selectors
            .iter()
            .map(|selector| resolve(atlas, selector))
            .collect::<Result<_, _>>()?
    };
    if devices.is_empty() {
        return Err("choose devices to update, or pass --all".into());
    }

    let mut releases = BTreeMap::new();
    for entry in &options.releases {
        let (family, version) = entry
            .split_once('=')
            .filter(|(family, version)| !family.trim().is_empty() && !version.trim().is_empty())
            .ok_or_else(|| format!("--release expects `family=version`, got `{entry}`"))?;
        releases.insert(Family::new(family.trim()), version.trim().to_string());
    }
    if let Some(version) = &options.version {
        for key in &devices {
            releases
                .entry(key.family.clone())
                .or_insert_with(|| version.clone());
        }
    }
    if releases.is_empty() {
        return Err("choose a version with --version or --release family=version".into());
    }
    Ok(UpdateRequest {
        devices,
        releases,
        staged: options.staged,
    })
}

fn print_plan(plan: &JobPlan) {
    let rows: Vec<Vec<String>> = plan
        .devices
        .iter()
        .map(|device| {
            let order = match (&device.plan.concurrency, device.canary) {
                (_, true) => "first (staged)".to_string(),
                (Concurrency::Parallel, false) => "parallel".to_string(),
                (Concurrency::Exclusive(resource), false) => format!("one at a time ({resource})"),
            };
            vec![
                device.name.clone(),
                device.from_version.clone().unwrap_or_else(|| "-".into()),
                device.release.version.clone(),
                order,
                device.plan.summary.clone(),
            ]
        })
        .collect();
    output::table(&["DEVICE", "FROM", "TO", "ORDER", "PLAN"], &rows);
}

pub(crate) async fn update(atlas: &Atlas, options: UpdateOptions) -> CommandResult {
    atlas.scan().await;
    let request = build_request(atlas, &options)?;
    let plan = atlas
        .plan_update(&request)
        .map_err(|error| error.to_string())?;
    if options.json {
        output::json(&json!({ "plan": plan }));
    } else {
        print_plan(&plan);
    }
    if options.dry_run {
        return Ok(ExitCode::SUCCESS);
    }

    let names: HashMap<DeviceKey, String> = plan
        .devices
        .iter()
        .map(|device| (device.device.clone(), device.name.clone()))
        .collect();
    let mut events = atlas.subscribe();
    let job = atlas
        .start_update(request)
        .map_err(|error| error.to_string())?;
    if !options.json {
        output::line(&format!("\nStarted {job}. Press Ctrl+C to cancel."));
    }

    let canceller = atlas.clone();
    let ctrl_c = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = canceller.cancel_job(job);
        }
    });

    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
        let finished = matches!(&event, Event::JobFinished { job: id, .. } if *id == job);
        if options.json {
            if event_job(&event) == Some(job)
                && let Ok(value) = serde_json::to_value(&event)
            {
                output::json(&value);
            }
        } else if let Some(text) = describe(&event, job, &names) {
            output::line(&text);
        }
        if finished {
            break;
        }
    }
    ctrl_c.abort();

    let record = atlas
        .wait_job(job)
        .await
        .map_err(|error| error.to_string())?;
    let summary = record.summary.unwrap_or_default();
    if !options.json {
        output::line(&format!(
            "\n{} verified, {} rolled back, {} need recovery, {} failed, {} skipped, {} cancelled",
            summary.verified,
            summary.rolled_back,
            summary.needs_recovery,
            summary.failed,
            summary.skipped,
            summary.cancelled
        ));
    }
    Ok(if summary.all_verified() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn event_job(event: &Event) -> Option<atlas_core::JobId> {
    match event {
        Event::JobStarted { job, .. }
        | Event::JobDevice { job, .. }
        | Event::JobStep { job, .. }
        | Event::JobProgress { job, .. }
        | Event::JobLog { job, .. }
        | Event::JobFinished { job, .. } => Some(*job),
        _ => None,
    }
}

fn describe(
    event: &Event,
    job: atlas_core::JobId,
    names: &HashMap<DeviceKey, String>,
) -> Option<String> {
    if event_job(event) != Some(job) {
        return None;
    }
    let name = |key: &DeviceKey| names.get(key).cloned().unwrap_or_else(|| key.to_string());
    match event {
        Event::JobStep { device, step, .. } => Some(format!("[{}] {}", name(device), step.label())),
        Event::JobLog {
            device, message, ..
        } => Some(format!("[{}] {message}", name(device))),
        Event::JobDevice { device, status, .. } => {
            let detail = match status {
                DeviceJobStatus::Queued | DeviceJobStatus::Running => return None,
                DeviceJobStatus::Verified { version } => format!("verified on {version}"),
                DeviceJobStatus::RolledBack { reason } => format!("rolled back: {reason}"),
                DeviceJobStatus::NeedsRecovery { reason } => format!("needs recovery: {reason}"),
                DeviceJobStatus::Failed { error } => format!("failed: {error}"),
                DeviceJobStatus::Skipped { reason } => format!("skipped: {reason}"),
                DeviceJobStatus::Cancelled => "cancelled".into(),
            };
            Some(format!("[{}] {detail}", name(device)))
        }
        _ => None,
    }
}

pub(crate) async fn action(
    atlas: &Atlas,
    selector: &str,
    action: Option<&str>,
    as_json: bool,
) -> CommandResult {
    atlas.scan().await;
    let key = resolve(atlas, selector)?;
    let Some(action) = action else {
        let actions = atlas.actions(&key).map_err(|error| error.to_string())?;
        if as_json {
            output::json(&json!({ "device": key, "actions": actions }));
        } else {
            let rows: Vec<Vec<String>> = actions
                .iter()
                .map(|action| {
                    let note = if action.destructive {
                        "destructive"
                    } else {
                        ""
                    };
                    vec![action.id.clone(), action.label.clone(), note.to_string()]
                })
                .collect();
            output::table(&["ACTION", "LABEL", ""], &rows);
        }
        return Ok(ExitCode::SUCCESS);
    };
    atlas
        .run_action(&key, action)
        .await
        .map_err(|error| error.to_string())?;
    if as_json {
        output::json(&json!({ "device": key, "action": action, "ok": true }));
    } else {
        output::line(&format!("Ran {action} on {key}"));
    }
    Ok(ExitCode::SUCCESS)
}
