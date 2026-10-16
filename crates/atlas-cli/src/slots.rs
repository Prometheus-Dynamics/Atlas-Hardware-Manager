//! `atlas slots`: a slots board's installed OS images (dynamic slots).

use std::path::PathBuf;
use std::process::ExitCode;

use atlas_core::Atlas;
use atlas_driver::{ProgressSink, ProgressUpdate, SlotCommand, SlotInstall, SlotSet};
use clap::Subcommand;
use serde_json::json;

use crate::commands::{CommandResult, local_artifact, resolve};
use crate::output;

#[derive(Subcommand)]
pub(crate) enum SlotsCommand {
    /// List the installed slots and the free space (the default).
    List,
    /// Install a slot bundle (<id>.slot.tar).
    Install {
        bundle: PathBuf,
        /// Keep it from removal.
        #[arg(long)]
        pin: bool,
        /// Boot it once when it is in (on trial: the board goes back by
        /// itself when it doesn't come up).
        #[arg(long)]
        boot: bool,
    },
    /// Restart into a slot once, on trial.
    BootOnce {
        id: String,
    },
    /// Make a slot that booted well the default.
    SetDefault {
        id: String,
    },
    /// Remove a slot (not the running, default, trial or a pinned one).
    Remove {
        id: String,
        /// Needed: removing deletes the image from the board.
        #[arg(long)]
        yes: bool,
    },
    Pin {
        id: String,
    },
    Unpin {
        id: String,
    },
}

pub(crate) async fn slots(
    atlas: &Atlas,
    selector: &str,
    command: Option<SlotsCommand>,
    as_json: bool,
) -> CommandResult {
    atlas.scan().await;
    let key = resolve(atlas, selector)?;
    let command = match command.unwrap_or(SlotsCommand::List) {
        SlotsCommand::List => {
            let set = atlas
                .slots(&key)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("{key} has no slots (an A/B board?)"))?;
            if as_json {
                output::json(&json!({ "device": key, "slots": set }));
            } else {
                print(&set);
            }
            return Ok(ExitCode::SUCCESS);
        }
        SlotsCommand::Install { bundle, pin, boot } => {
            let bundle = local_artifact(&bundle).await?;
            let install = SlotInstall { bundle, pin, boot };
            let quiet = as_json;
            // Each tenth of the transfer, and the board's notes.
            let shown = std::sync::atomic::AtomicU32::new(0);
            let progress = ProgressSink::new(move |update| {
                if quiet {
                    return;
                }
                match update {
                    ProgressUpdate::StepProgress { fraction, .. } => {
                        let tenth = (fraction * 10.0).floor() as u32;
                        if tenth > shown.fetch_max(tenth, std::sync::atomic::Ordering::Relaxed) {
                            output::line(&format!("{:>3}%", tenth * 10));
                        }
                    }
                    ProgressUpdate::Log { message } => output::line(&message),
                    ProgressUpdate::StepStarted { .. } => {}
                }
            });
            atlas
                .install_slot(&key, &install, &progress)
                .await
                .map_err(|error| error.to_string())?;
            if as_json {
                output::json(
                    &json!({ "device": key, "installed": install.bundle.name, "ok": true }),
                );
            }
            return Ok(ExitCode::SUCCESS);
        }
        SlotsCommand::BootOnce { id } => SlotCommand::BootOnce { id },
        SlotsCommand::SetDefault { id } => SlotCommand::SetDefault { id },
        SlotsCommand::Remove { id, yes } => {
            if !yes {
                return Err(format!(
                    "removing deletes {id} from the board; run it again with --yes"
                ));
            }
            SlotCommand::Remove { id }
        }
        SlotsCommand::Pin { id } => SlotCommand::Pin { id, pinned: true },
        SlotsCommand::Unpin { id } => SlotCommand::Pin { id, pinned: false },
    };
    atlas
        .slot_command(&key, &command)
        .await
        .map_err(|error| error.to_string())?;
    if as_json {
        output::json(&json!({ "device": key, "command": command, "ok": true }));
    } else {
        output::line(&format!("{} on {key}: done", command.describe()));
    }
    Ok(ExitCode::SUCCESS)
}

fn print(set: &SlotSet) {
    let rows: Vec<Vec<String>> = set
        .slots
        .iter()
        .map(|slot| {
            let mut flags = slot.flags.clone();
            if slot.pinned {
                flags.push("pinned".into());
            }
            if slot.good == "no" {
                flags.push("failed".into());
            }
            vec![
                slot.id.clone(),
                slot.family.clone(),
                slot.version.clone(),
                slot.variant.clone(),
                mib(slot.size),
                if slot.verity { "yes" } else { "no" }.into(),
                flags.join(","),
            ]
        })
        .collect();
    output::table(
        &["SLOT", "OS", "VERSION", "VARIANT", "SIZE", "VERIFIED", ""],
        &rows,
    );
    output::line(&format!(
        "{} free for slots ({} kept free)",
        mib(set.room()),
        mib(set.reserve_bytes)
    ));
}

fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes >> 20)
}
