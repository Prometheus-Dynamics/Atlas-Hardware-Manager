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
    /// Use the file even though it does not match its expected SHA-256.
    /// Atlas never blocks a deliberate choice; it only stops by default.
    #[serde(default)]
    pub ignore_checksum: bool,
    /// A file to use once, without adding it to the release list.
    #[serde(default)]
    pub path: Option<String>,
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
                            ignore_checksum: false,
                            path: None,
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
/// (planning) the catalog's metadata is used as-is. Unsigned files are
/// allowed; the UI shows them as unsigned. A checksum mismatch stops unless
/// the choice says `ignore_checksum`.
async fn resolve(
    app: &AppHandle,
    state: &AppState,
    input: UpdateRequestInput,
    fetch: bool,
) -> CmdResult<UpdateRequest> {
    let mut releases = BTreeMap::new();
    for (family, choice) in input.releases {
        if let Some(path) = choice
            .path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
        {
            let path = std::path::Path::new(path);
            // Planning only needs the name and size; starting hashes it so
            // the write is still verified against the file.
            let artifact = if fetch {
                atlas_release::local_artifact(path).await.map_err(text)?
            } else {
                let metadata = std::fs::metadata(path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                Artifact {
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "image".into()),
                    path: path.to_path_buf(),
                    sha256: String::new(),
                    size_bytes: metadata.len(),
                }
            };
            releases.insert(
                family,
                ReleaseTarget {
                    version: choice.version,
                    artifact: Some(artifact),
                },
            );
            continue;
        }
        let artifact = match &choice.release_id {
            None => None,
            Some(id) => {
                let entry = state
                    .releases
                    .entry(id)
                    .ok_or_else(|| ReleaseError::UnknownRelease(id.clone()).to_string())?;
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
                let downloaded = state
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
                    .await;
                let checked = match downloaded {
                    Ok(_) => state.releases.artifact(id).await,
                    Err(error) => Err(error),
                };
                Some(match checked {
                    Ok(artifact) => artifact,
                    Err(ReleaseError::HashMismatch { .. }) if choice.ignore_checksum => {
                        state.releases.artifact_unchecked(id).await.map_err(text)?
                    }
                    Err(ReleaseError::HashMismatch {
                        expected, actual, ..
                    }) => {
                        return Err(format!(
                            "{} failed its SHA-256 check (expected {expected}, got {actual}). \
                             The download may be corrupt: download it again, or choose \
                             Flash anyway to use it as it is.",
                            entry.artifact_name
                        ));
                    }
                    Err(error) => return Err(error.to_string()),
                })
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

#[tauri::command]
pub fn get_job(state: State<'_, AppState>, id: JobId) -> Option<JobRecord> {
    state.atlas.job(id)
}
