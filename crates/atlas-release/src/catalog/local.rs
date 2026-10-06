//! Local image files: keeping the list short, and one-time use.

use std::path::Path;

use atlas_driver::Artifact;

use super::{Channel, ReleaseEntry};
use crate::{ReleaseError, sha256_file};

/// Unpinned local images kept in the list; older ones drop off (the files
/// themselves are never touched).
pub const KEEP_LOCAL: usize = 5;

pub(crate) fn prune_local(entries: &mut Vec<ReleaseEntry>) {
    let mut unpinned: Vec<(u64, String)> = entries
        .iter()
        .filter(|entry| entry.channel == Channel::Local && !entry.pinned)
        .map(|entry| (entry.added_ms, entry.id.clone()))
        .collect();
    if unpinned.len() <= KEEP_LOCAL {
        return;
    }
    unpinned.sort_by(|a, b| b.0.cmp(&a.0));
    let drop: Vec<String> = unpinned
        .into_iter()
        .skip(KEEP_LOCAL)
        .map(|(_, id)| id)
        .collect();
    entries.retain(|entry| !drop.contains(&entry.id));
}

/// A file used once, without adding it to the catalog: hashed now so the
/// write is still checked against it.
pub async fn local_artifact(path: &Path) -> Result<Artifact, ReleaseError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| ReleaseError::io(path, error))?;
    Ok(Artifact {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".into()),
        path: path.to_path_buf(),
        sha256: sha256_file(path).await?,
        size_bytes: metadata.len(),
    })
}
