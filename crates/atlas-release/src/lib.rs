//! Releases Atlas can install: local image files and signed remote
//! manifests, with SHA-256 checks on every file before it is used.
//!
//! A remote release is trusted only when its manifest carries a valid
//! ed25519 signature from a pinned key. A local file is marked unsigned and
//! the UI asks the user to opt in before installing it.

mod catalog;
mod error;
mod hash;
mod manifest;

pub use catalog::{
    Channel, DownloadProgress, KEEP_LOCAL, ReleaseCatalog, ReleaseEntry, ReleaseOrigin,
    RemoteSource, local_artifact,
};
pub use error::ReleaseError;
pub use hash::{normalize_sha256, sha256_file};
pub use manifest::{
    Manifest, ManifestArtifact, ManifestBody, PublicKey, Requirements, public_key_for,
    sign_manifest,
};
