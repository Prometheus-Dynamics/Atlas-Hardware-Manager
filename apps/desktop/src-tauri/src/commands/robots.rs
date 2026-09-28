use atlas_core::{RobotProfile, RobotStatus, StagedRollout};
use tauri::State;

use super::jobs::UpdateRequestInput;
use super::{CmdResult, text};
use crate::state::AppState;

#[tauri::command]
pub fn list_robots(state: State<'_, AppState>) -> Vec<RobotProfile> {
    state.atlas.robots()
}

#[tauri::command]
pub fn robot_statuses(state: State<'_, AppState>) -> Vec<RobotStatus> {
    state.atlas.robot_statuses()
}

/// Creates or replaces a profile. Pass `previous_name` to rename one.
#[tauri::command]
pub fn save_robot(
    state: State<'_, AppState>,
    profile: RobotProfile,
    previous_name: Option<String>,
) -> CmdResult<()> {
    state
        .atlas
        .save_robot(profile, previous_name.as_deref())
        .map_err(text)
}

#[tauri::command]
pub fn delete_robot(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.atlas.delete_robot(&name).map_err(text)
}

/// The update that would bring a robot to its targets, ready for
/// `plan_update` or `start_update`. `None` when it is already up to date.
#[tauri::command]
pub fn robot_update_request(
    state: State<'_, AppState>,
    name: String,
    staged: Option<StagedRollout>,
) -> CmdResult<Option<UpdateRequestInput>> {
    let status = state.atlas.robot_status(&name).map_err(text)?;
    let staged = staged.unwrap_or(state.settings().staged_default);
    Ok(status
        .update_request(staged)
        .map(UpdateRequestInput::from_request))
}
