//! The release catalog: first-flash images (`.img.xz`) known to this
//! computer, signed remote manifests, and downloads checked before use.
//!
//! Security is one switch ([`Security`]), off by default. Off: an unsigned
//! image is allowed and a bad or unknown signature is reported, and every
//! file is still SHA-256 checked when a hash is known. On: a file or
//! manifest without a good signature from a trusted, unexpired key is
//! refused.
//!
//! Signatures use the schemes Atlas already uses: minisign for image files
//! (`<file>.minisig`, checked against [`Keyring`]) and ed25519 over the
//! compact JSON of a [`ManifestBody`] for remote releases.

mod catalog;
mod error;
mod fetch;
mod hash;
mod manifest;
mod trust;

use std::time::{SystemTime, UNIX_EPOCH};

pub use catalog::{
    Artifact, Channel, DownloadProgress, ReleaseCatalog, ReleaseEntry, ReleaseOrigin, RemoteSource,
};
pub use error::ReleaseError;
pub use fetch::{Body, Fetch, HttpFetcher, Reply};
pub use hash::{normalize_sha256, sha256_file};
pub use manifest::{
    Manifest, ManifestArtifact, ManifestBody, PublicKey, Requirements, public_key_for,
    sign_manifest,
};
pub use trust::{
    Keyring, Purpose, Security, SignatureStatus, TrustedKey, check_file, signature_path,
};

/// The only image kind: a first-flash image, xz-compressed.
pub const IMAGE_SUFFIX: &str = ".img.xz";

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
