use std::collections::BTreeMap;

use atlas_core::{JobId, JobPlan, JobRecord, ReleaseTarget, StagedRollout, UpdateRequest};
use atlas_driver::{Artifact, DeviceKey, Family};
use atlas_release::ReleaseError;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::releases::DownloadEvent;
use super::{CmdResult, text};
use crate::state::AppState;

/// The release chosen for one family. `release_id` points at the catalog
/// entry whose file is installed; devices that fetch their own updates
/// need only `version`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseChoice {
    pub version: String,
    #[serde(default)]
    pub release_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateRequestInput {
    pub devices: Vec<DeviceKey>,
    pub releases: BTreeMap<Family, ReleaseChoice>,
    #[serde(default)]
    pub staged: StagedRollout,
}

impl UpdateRequestInput {
    pub fn from_request(request: UpdateRequest) -> Self {
        Self {
            devices: request.devices,
            releases: request
                .releases
                .into_iter()
                .map(|(family, target)| {
                    (
                        family,
                        ReleaseChoice {
                            version: target.version,
                            release_id: None,
                        },
                    )
                })
                .collect(),
            staged: request.staged,
        }
    }
}

/// Turns release ids into artifacts. With `fetch`, remote releases are
/// downloaded and every file is re-checked against its SHA-256; without it
/// (planning) the catalog's metadata is used as-is. Unsigned local files
/// are refused unless the user allowed them.
async fn resolve(
    app: &AppHandle,
    state: &AppState,
    input: UpdateRequestInput,
    fetch: bool,
) -> CmdResult<UpdateRequest> {
    let allow_unsigned = state.settings().allow_unsigned_local;
    let mut releases = BTreeMap::new();
    for (family, choice) in input.releases {
        let artifact = match &choice.release_id {
            None => None,
            Some(id) => {
                let entry = state
                    .releases
                    .entry(id)
                    .ok_or_else(|| ReleaseError::UnknownRelease(id.clone()).to_string())?;
                if !entry.signed && !allow_unsigned {
                    return Err(format!(
                        "{} is an unsigned local file. Allow unsigned local files in Settings to install it.",
                        entry.artifact_name
                    ));
                }
                if !fetch {
                    releases.insert(
                        family,
                        ReleaseTarget {
                            version: choice.version,
                            artifact: Some(Artifact {
                                name: entry.artifact_name,
                                path: entry.path.unwrap_or_default(),
                                sha256: entry.sha256,
                                size_bytes: entry.size_bytes,
                            }),
                        },
                    );
                    continue;
                }
                let id_for_events = id.clone();
                let app_for_events = app.clone();
                state
                    .releases
                    .download(id, move |progress| {
                        let _ = app_for_events.emit(
                            "atlas://download",
                            DownloadEvent {
                                id: id_for_events.clone(),
                                downloaded: progress.downloaded,
                                total: progress.total,
                            },
                        );
                    })
                    .await
                    .map_err(text)?;
                Some(state.releases.artifact(id).await.map_err(text)?)
            }
        };
        releases.insert(
            family,
            ReleaseTarget {
                version: choice.version,
                artifact,
            },
        );
    }
    Ok(UpdateRequest {
        devices: input.devices,
        releases,
        staged: input.staged,
    })
}

#[tauri::command]
pub async fn plan_update(
    app: AppHandle,
    state: State<'_, AppState>,
    request: UpdateRequestInput,
) -> CmdResult<JobPlan> {
    let request = resolve(&app, &state, request, false).await?;
    state.atlas.plan_update(&request).map_err(text)
}

#[tauri::command]
pub async fn start_update(
    app: AppHandle,
    state: State<'_, AppState>,
    request: UpdateRequestInput,
) -> CmdResult<JobId> {
    let request = resolve(&app, &state, request, true).await?;
    state.atlas.start_update(request).map_err(text)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, id: JobId) -> CmdResult<()> {
    state.atlas.cancel_job(id).map_err(text)
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> Vec<JobRecord> {
    state.atlas.jobs()
}
