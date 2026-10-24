//! `atlas slots`: a slots board's installed OS images (dynamic slots).

use std::path::PathBuf;
use std::process::ExitCode;

use atlas_core::Atlas;
use atlas_driver::{
    LayerPush, LayerPushed, ProgressSink, ProgressUpdate, PushMode, SlotCommand, SlotInstall,
    SlotSet,
};
use clap::Subcommand;
use serde_json::json;

use crate::commands::{CommandResult, local_artifact, resolve};
use crate::output;

#[derive(Subcommand)]
pub(crate) enum SlotsCommand {
    /// List the installed slots and the free space (the default).
    List,
    /// Install a slot bundle (<id>.slot.tar), or a slots build's image
    /// (<id>.img.xz): the bundle is made from it first.
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
    BootOnce { id: String },
    /// Make a slot that booted well the default.
    SetDefault { id: String },
    /// Remove a slot (not the running, default, trial or a pinned one).
    Remove {
        id: String,
        /// Needed: removing deletes the image from the board.
        #[arg(long)]
        yes: bool,
    },
    /// Keep a slot: `remove` refuses it until it is unpinned
    Pin { id: String },
    /// Let a pinned slot be removed again
    Unpin { id: String },
    /// Replace one layer of the running layered slot (the app layer, in
    /// seconds): laid over the running system and its units restarted, and
    /// kept for the next boot.
    Push {
        /// The layer image (<id>.erofs), or a ready push archive (.tar with
        /// layer.json and <id>.erofs).
        layer: Option<PathBuf>,
        /// The build's layers.json (or the layer's own entry), for an image.
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Which layer of the slot.
        #[arg(long, default_value = "app")]
        name: String,
        /// Restart the board into it instead of laying it over now.
        #[arg(long, conflicts_with = "no_restart")]
        reboot: bool,
        /// Only keep it for the next boot.
        #[arg(long)]
        no_restart: bool,
        /// Put back the layer the last push replaced.
        #[arg(long, conflicts_with_all = ["layer", "manifest"])]
        revert: bool,
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
            let bundle = if atlas_blockdev::is_slot_bundle(&bundle) {
                bundle
            } else {
                if !as_json {
                    output::line(&format!("making the slot bundle from {}", bundle.display()));
                }
                let out = dirs::cache_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("atlas")
                    .join("slot-bundles");
                tokio::task::spawn_blocking(move || {
                    atlas_blockdev::slot_bundle_from_image(&bundle, &out)
                })
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?
                .path
            };
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
        SlotsCommand::Push {
            layer,
            manifest,
            name,
            reboot,
            no_restart,
            revert,
        } => {
            let mode = if reboot {
                PushMode::Reboot
            } else if no_restart {
                PushMode::Keep
            } else {
                PushMode::Live
            };
            let pushed = push(atlas, &key, layer, manifest, name, mode, revert, as_json).await?;
            if as_json {
                output::json(&json!({ "device": key, "pushed": pushed, "ok": true }));
            } else {
                print_pushed(&pushed);
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
        .map_err(|error| format!("can't {}: {error}", command.describe()))?;
    if as_json {
        output::json(&json!({ "device": key, "command": command, "ok": true }));
    } else {
        output::line(&format!("{} on {key}: done", command.describe()));
    }
    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
async fn push(
    atlas: &Atlas,
    key: &atlas_driver::DeviceKey,
    layer: Option<PathBuf>,
    manifest: Option<PathBuf>,
    name: String,
    mode: PushMode,
    revert: bool,
    quiet: bool,
) -> Result<LayerPushed, String> {
    use crate::layer_push;
    let progress = ProgressSink::new(move |update| {
        if !quiet && let ProgressUpdate::Log { message } = update {
            output::line(&message);
        }
    });
    if revert {
        let push = LayerPush {
            layer: name,
            archive: PathBuf::new(),
            size_bytes: 0,
            mode,
            revert: true,
        };
        return atlas
            .push_layer(key, &push, &progress)
            .await
            .map_err(|error| error.to_string());
    }
    let layer = layer.ok_or("a layer image (or a push archive) to push")?;
    let is_archive = layer.extension().is_some_and(|ext| ext == "tar");
    let temp = layer_push::temp_archive();
    let archive = if is_archive {
        layer.clone()
    } else {
        let manifest = manifest
            .ok_or("--manifest: the build's layers.json (or the layer's entry) for the image")?;
        let text = std::fs::read_to_string(&manifest)
            .map_err(|error| format!("{}: {error}", manifest.display()))?;
        let entry =
            layer_push::entry(&text, &name).map_err(|e| format!("{}: {e}", manifest.display()))?;
        // The board checks it too; checking here saves sending a wrong one.
        let local = local_artifact(&layer).await?;
        let want = entry["sha256"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if local.sha256.to_ascii_lowercase() != want {
            return Err(format!(
                "{} doesn't match the {name} layer's sha256 in {}",
                layer.display(),
                manifest.display()
            ));
        }
        layer_push::write_archive(&entry, &layer, &temp)?;
        temp.clone()
    };
    let size_bytes = std::fs::metadata(&archive)
        .map_err(|error| format!("{}: {error}", archive.display()))?
        .len();
    let push = LayerPush {
        layer: name,
        archive,
        size_bytes,
        mode,
        revert: false,
    };
    let result = atlas.push_layer(key, &push, &progress).await;
    if !is_archive {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|error| error.to_string())
}

fn print_pushed(pushed: &LayerPushed) {
    if !pushed.changed {
        output::line(&format!(
            "{} already runs that {} layer",
            pushed.slot, pushed.layer
        ));
        return;
    }
    let short = |id: &str| id.chars().take(12).collect::<String>();
    output::line(&format!(
        "{}: {} layer {} (was {})",
        pushed.slot,
        pushed.layer,
        short(&pushed.id),
        short(&pushed.previous)
    ));
    if !pushed.live.is_empty() {
        output::line(&format!("  live now: {}", pushed.live.join(" ")));
    }
    if !pushed.restarted.is_empty() {
        output::line(&format!("  restarted: {}", pushed.restarted.join(" ")));
    }
    if !pushed.next_boot.is_empty() {
        output::line(&format!(
            "  at the next boot: {}",
            pushed.next_boot.join(" ")
        ));
    }
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
                if !slot.intact {
                    "DAMAGED"
                } else if slot.verity {
                    "yes"
                } else {
                    "no"
                }
                .into(),
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
