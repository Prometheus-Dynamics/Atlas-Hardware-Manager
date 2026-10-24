//! A slots board's installed OS images (dynamic slots).

use std::path::PathBuf;

use atlas_driver::{
    Artifact, DeviceKey, ProgressSink, ProgressUpdate, SlotCommand, SlotInstall, SlotSet,
};
use tauri::State;
use tauri::ipc::Channel;

use super::{CmdResult, text};
use crate::state::AppState;

/// The board's slots; `null` when it has none.
#[tauri::command]
pub async fn device_slots(
    state: State<'_, AppState>,
    key: DeviceKey,
) -> CmdResult<Option<SlotSet>> {
    state.atlas.slots(&key).await.map_err(text)
}

#[tauri::command]
pub async fn slot_command(
    state: State<'_, AppState>,
    key: DeviceKey,
    command: SlotCommand,
) -> CmdResult<()> {
    state.atlas.slot_command(&key, &command).await.map_err(text)
}

/// Installs the slot bundle at `path` (hashed here first), reporting the
/// transfer on `on_progress`. `expected_sha256`, the chosen release's digest
/// when it came from the catalog, must match the file: a cached or picked
/// file that isn't that release is refused before anything is sent. A slots
/// build's `.img.xz` (what a fresh install writes) installs too: the bundle
/// is made from it first.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn install_slot(
    state: State<'_, AppState>,
    key: DeviceKey,
    path: PathBuf,
    pin: bool,
    boot: bool,
    expected_sha256: Option<String>,
    on_progress: Channel<ProgressUpdate>,
) -> CmdResult<()> {
    let sha256 = atlas_release::sha256_file(&path).await.map_err(text)?;
    if let Some(want) = expected_sha256.filter(|want| !want.is_empty())
        && !want.eq_ignore_ascii_case(&sha256)
    {
        return Err(format!(
            "{} isn't the release you chose: its SHA-256 is {}…, the release's is {}…; nothing was sent",
            path.display(),
            sha256.get(..12).unwrap_or(&sha256),
            want.get(..12).unwrap_or(&want)
        ));
    }
    let progress = ProgressSink::new(move |update| {
        let _ = on_progress.send(update);
    });
    let (path, sha256) = if atlas_blockdev::is_slot_bundle(&path) {
        (path, sha256)
    } else {
        progress.log(format!(
            "making the slot bundle from {}",
            path.file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
        ));
        let out = state.paths.cache_dir.join("slot-bundles");
        let made = tokio::task::spawn_blocking(move || {
            atlas_blockdev::slot_bundle_from_image(&path, &out)
        })
        .await
        .map_err(text)?
        .map_err(text)?;
        let sha256 = atlas_release::sha256_file(&made.path).await.map_err(text)?;
        (made.path, sha256)
    };
    let size_bytes = std::fs::metadata(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    let install = SlotInstall {
        bundle: Artifact {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "slot".into()),
            path,
            sha256,
            size_bytes,
        },
        pin,
        boot,
    };
    state
        .atlas
        .install_slot(&key, &install, &progress)
        .await
        .map_err(text)
}
