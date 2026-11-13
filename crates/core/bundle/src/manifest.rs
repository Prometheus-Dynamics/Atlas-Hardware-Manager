//! `slot.json`, schema 2: the manifest every slot bundle carries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::BundleError;

/// The manifest's schema.
pub const SCHEMA: u32 = 2;
/// The boot file every slot must have.
pub const CMDLINE: &str = "cmdline.txt";

/// What `list` shows for a slot, and what the manifest carries beside the
/// fields a writer must have. Absent fields are left out of the JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotMeta {
    pub family: Option<String>,
    pub version: Option<String>,
    pub variant: Option<String>,
    pub kernel: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotManifest {
    pub schema: u32,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub image: ImageEntry,
    pub verity: VerityEntry,
    pub boot: BootEntry,
}

/// `rootfs.erofs`: its length and SHA-256 (the file, tree included).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageEntry {
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerityEntry {
    pub root_hash: String,
}

/// `boot/...`: each file's path under `boot/` to its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootEntry {
    pub files: BTreeMap<String, String>,
}

impl SlotManifest {
    pub(crate) fn new(
        id: &str,
        meta: &SlotMeta,
        image: ImageEntry,
        root_hash: String,
        boot: BTreeMap<String, String>,
    ) -> Self {
        Self {
            schema: SCHEMA,
            id: id.to_string(),
            family: meta.family.clone(),
            version: meta.version.clone(),
            variant: meta.variant.clone(),
            kernel: meta.kernel.clone(),
            display_name: meta.display_name.clone(),
            image,
            verity: VerityEntry { root_hash },
            boot: BootEntry { files: boot },
        }
    }

    /// The bytes `slot.json` is written as, and that `slot.sig` signs.
    pub fn to_json(&self) -> Result<Vec<u8>, BundleError> {
        serde_json::to_vec(self).map_err(|error| BundleError::Malformed(error.to_string()))
    }

    /// Checks what a reader needs before it trusts the fields.
    pub fn validate(&self) -> Result<(), BundleError> {
        if self.schema != SCHEMA {
            return Err(BundleError::Malformed(format!(
                "slot.json schema {} is not {SCHEMA}",
                self.schema
            )));
        }
        validate_id(&self.id)?;
        if !is_hex(&self.image.sha256, 64) {
            return Err(BundleError::Malformed(
                "image.sha256 is not a SHA-256 digest".into(),
            ));
        }
        if !is_hex(&self.verity.root_hash, 64) {
            return Err(BundleError::Malformed(
                "verity.root_hash is not a SHA-256 digest".into(),
            ));
        }
        for (path, digest) in &self.boot.files {
            validate_boot_path(path)?;
            if !is_hex(digest, 64) {
                return Err(BundleError::Malformed(format!(
                    "boot file {path} has no SHA-256 digest"
                )));
            }
        }
        if !self.boot.files.contains_key(CMDLINE) {
            return Err(BundleError::MissingCmdline);
        }
        Ok(())
    }
}

/// A slot id: 1 to 64 of `[A-Za-z0-9._-]`, not `.` or `..`.
pub fn validate_id(id: &str) -> Result<(), BundleError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    ok.then_some(())
        .ok_or_else(|| BundleError::InvalidId(id.to_string()))
}

/// A boot file path under `boot/`: relative, plain names of
/// `[A-Za-z0-9._+-]`, no empty, `.` or `..` part.
pub fn validate_boot_path(path: &str) -> Result<(), BundleError> {
    let bad = || BundleError::InvalidBootPath(path.to_string());
    if path.is_empty() || path.len() > 512 || path.starts_with('/') {
        return Err(bad());
    }
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
        {
            return Err(bad());
        }
    }
    Ok(())
}

/// `len` hex digits, either case.
pub(crate) fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}
