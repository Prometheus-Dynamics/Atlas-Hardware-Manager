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
/// transfer on `on_progress`.
#[tauri::command]
pub async fn install_slot(
    state: State<'_, AppState>,
    key: DeviceKey,
    path: PathBuf,
    pin: bool,
    boot: bool,
    on_progress: Channel<ProgressUpdate>,
) -> CmdResult<()> {
    let size_bytes = std::fs::metadata(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    let sha256 = atlas_release::sha256_file(&path).await.map_err(text)?;
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
    let progress = ProgressSink::new(move |update| {
        let _ = on_progress.send(update);
    });
    state
        .atlas
        .install_slot(&key, &install, &progress)
        .await
        .map_err(text)
}
