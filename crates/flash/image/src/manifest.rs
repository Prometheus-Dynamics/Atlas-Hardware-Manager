//! Lemnos's slot files on the data partition, as Gaia writes them:
//!
//! - `slots/<id>.json`: the slot's manifest (the bundle's `slot.json` form). The
//!   reader takes `id`, `image.size`, `image.sha256`, `verity.root_hash` and
//!   `boot.files` (path under `boot/` to sha256). Other fields are not read.
//! - `slots/<id>.boot`: one `<sha256>  <path>` line per boot file, the
//!   `sha256sum -c` form, including `slot.json`.
//! - `slots/bootfiles/<sha256>`: the boot files, kept by content.
//!
//! Nothing here trusts a value because it is written down: every boot file is
//! hashed against its name, and the manifest's boot list must agree with the
//! `.boot` list.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::ImageError;
use crate::names;

/// The manifest fields the reader takes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) id: String,
    pub(crate) image: ImageField,
    pub(crate) verity: VerityField,
    #[serde(default)]
    pub(crate) boot: BootField,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct ImageField {
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct VerityField {
    pub(crate) root_hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct BootField {
    #[serde(default)]
    pub(crate) files: BTreeMap<String, String>,
}

/// The most boot-list lines read: a slot has a few dozen files.
const MAX_BOOT_LINES: usize = 4096;

fn bad(what: impl Into<String>) -> ImageError {
    ImageError::Malformed(what.into())
}

/// 64 lowercase hex digits.
pub(crate) fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Reads the manifest text of slot `id` and checks what the reader relies on.
pub(crate) fn parse_manifest(text: &str, id: &str) -> Result<Manifest, ImageError> {
    let manifest: Manifest = serde_json::from_str(text)
        .map_err(|error| bad(format!("slot {id:?}'s manifest: {error}")))?;
    if manifest.id != id {
        return Err(bad(format!(
            "slot {id:?}'s manifest says it is slot {:?}",
            manifest.id
        )));
    }
    if !is_sha256(&manifest.image.sha256) {
        return Err(bad(format!(
            "slot {id:?}'s image.sha256 is not a SHA-256 digest"
        )));
    }
    if !is_sha256(&manifest.verity.root_hash) {
        return Err(bad(format!(
            "slot {id:?}'s verity.root_hash is not a SHA-256 digest"
        )));
    }
    for (path, sha) in &manifest.boot.files {
        names::split_rel(path)?;
        if !is_sha256(sha) {
            return Err(bad(format!(
                "slot {id:?}: boot file {path:?} has no SHA-256"
            )));
        }
    }
    Ok(manifest)
}

/// One line of `.boot`: the digest and the path under `boot/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BootLine {
    pub(crate) sha256: String,
    pub(crate) path: String,
}

/// Parses `.boot`: `<sha256>  <path>` (or `<sha256> *<path>`, sha256sum's binary
/// mode). A path is checked like any name in the image; a path listed twice is
/// refused.
pub(crate) fn parse_boot_list(text: &str) -> Result<Vec<BootLine>, ImageError> {
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if lines.len() >= MAX_BOOT_LINES {
            return Err(ImageError::Limit(format!(
                "the boot list has more than {MAX_BOOT_LINES} lines"
            )));
        }
        let sha256 = line.get(..64).unwrap_or_default();
        let rest = line.get(64..).unwrap_or_default();
        let path = rest
            .strip_prefix("  ")
            .or_else(|| rest.strip_prefix(" *"))
            .ok_or_else(|| bad(format!("a boot list line has no path: {line:?}")))?;
        if !is_sha256(sha256) {
            return Err(bad(format!("a boot list line has no SHA-256: {line:?}")));
        }
        names::split_rel(path)?;
        if lines.iter().any(|other: &BootLine| other.path == path) {
            return Err(bad(format!("boot file {path:?} is listed twice")));
        }
        lines.push(BootLine {
            sha256: sha256.to_string(),
            path: path.to_string(),
        });
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_boot_list_reads_sha256sum_lines() {
        let text = format!("{DIGEST}  Image\n{DIGEST} *overlays/one.dtbo\n");
        let lines = parse_boot_list(&text).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].path, "overlays/one.dtbo");
    }

    #[test]
    fn a_boot_list_refuses_bad_paths_duplicates_and_digests() {
        assert!(parse_boot_list(&format!("{DIGEST}  ../x\n")).is_err());
        assert!(parse_boot_list(&format!("{DIGEST}  /etc/x\n")).is_err());
        assert!(parse_boot_list(&format!("{DIGEST}  a\n{DIGEST}  a\n")).is_err());
        assert!(parse_boot_list("nothex  Image\n").is_err());
        assert!(parse_boot_list(&format!("{DIGEST}Image\n")).is_err());
    }

    #[test]
    fn a_manifest_must_name_its_slot_and_hash_its_parts() {
        let good = format!(
            r#"{{"schema":2,"id":"alpha","image":{{"size":8,"sha256":"{DIGEST}"}},
                "verity":{{"root_hash":"{DIGEST}"}},"boot":{{"files":{{"Image":"{DIGEST}"}}}}}}"#
        );
        assert!(parse_manifest(&good, "alpha").is_ok());
        assert!(parse_manifest(&good, "beta").is_err());
        let short = good.replace(DIGEST, "abc");
        assert!(parse_manifest(&short, "alpha").is_err());
        assert!(parse_manifest("{", "alpha").is_err());
    }
}
