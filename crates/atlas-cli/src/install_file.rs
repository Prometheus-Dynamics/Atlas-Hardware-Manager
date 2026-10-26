//! `atlas install <device> <file>`: one file onto one device by the first
//! install method it can take it by (atlas-core's install_choices), or the
//! method named with `--method`. Fresh installs (they erase) go through
//! `atlas flash`, not here.

use std::path::PathBuf;
use std::process::ExitCode;

use atlas_core::Atlas;
use atlas_driver::{
    CancellationToken, InstallMethodId, InstallOptions, InstallRequest, ProgressSink,
    ProgressUpdate,
};
use serde_json::json;

use crate::commands::{CommandResult, local_artifact, resolve};
use crate::output;

pub(crate) struct InstallArgs {
    pub device: String,
    pub file: PathBuf,
    /// `slot`, `ab-image` or `package`.
    pub method: Option<String>,
    pub version: Option<String>,
    pub boot: bool,
    pub pin: bool,
    /// Say which method and what it would do; run nothing.
    pub dry_run: bool,
}

pub(crate) async fn install(atlas: &Atlas, args: InstallArgs, as_json: bool) -> CommandResult {
    let method: Option<InstallMethodId> = match &args.method {
        Some(name) => Some(
            serde_json::from_value(serde_json::Value::String(name.clone()))
                .map_err(|_| format!("`{name}` isn't a method: slot, ab-image or package"))?,
        ),
        None => None,
    };
    crate::commands::scan_settled(atlas).await;
    let key = resolve(atlas, &args.device)?;
    let artifact = local_artifact(&args.file).await?;
    let kind = {
        let path = args.file.clone();
        tokio::task::spawn_blocking(move || atlas_blockdev::sniff_artifact(&path))
            .await
            .map_err(|error| error.to_string())?
    };
    let version = args
        .version
        .clone()
        .unwrap_or_else(|| artifact.name.clone());
    let request = InstallRequest {
        artifact,
        kind,
        version,
        options: InstallOptions {
            boot_after: args.boot,
            pin: args.pin,
        },
    };
    let choices = atlas
        .install_choices(&key, &request)
        .map_err(|error| error.to_string())?;
    let chosen = match method {
        Some(method) => choices.choices.iter().find(|c| c.method == method),
        None => choices.choices.iter().find(|c| c.plan.is_some()),
    };
    let Some(chosen) = chosen.filter(|c| c.plan.is_some()) else {
        let why: Vec<String> = choices
            .choices
            .iter()
            .map(|c| {
                format!(
                    "{}: {}",
                    c.label,
                    c.why_not.as_deref().unwrap_or("not chosen")
                )
            })
            .collect();
        return Err(format!(
            "{key} can't take this {} that way{}",
            choices.kind_label,
            if why.is_empty() {
                String::new()
            } else {
                format!(" ({})", why.join("; "))
            }
        ));
    };
    if chosen.method == InstallMethodId::Fresh {
        return Err("a fresh install erases the device: use `atlas flash`".into());
    }
    if !as_json {
        output::line(&format!(
            "{}: {}",
            chosen.label,
            chosen
                .plan
                .as_ref()
                .map(|p| p.summary.as_str())
                .unwrap_or("")
        ));
    }
    if args.dry_run {
        if as_json {
            output::json(
                &json!({ "device": key, "kind": choices.kind, "choices": choices.choices }),
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    let quiet = as_json;
    let progress = ProgressSink::new(move |update| {
        if quiet {
            return;
        }
        match update {
            ProgressUpdate::StepStarted { step } => output::line(&format!("{}…", step.label())),
            ProgressUpdate::Log { message } => output::line(&message),
            ProgressUpdate::StepProgress { .. } => {}
        }
    });
    let method = chosen.method;
    match atlas
        .install_by(&key, method, &request, &progress, &CancellationToken::new())
        .await
    {
        Ok(_) => {
            if as_json {
                output::json(&json!({ "device": key, "method": method, "ok": true }));
            } else {
                output::line("done");
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(failure) => Err(format!(
            "{}{}",
            failure.error,
            if failure.changed_nothing {
                " (nothing on the device changed)"
            } else {
                ""
            }
        )),
    }
}
