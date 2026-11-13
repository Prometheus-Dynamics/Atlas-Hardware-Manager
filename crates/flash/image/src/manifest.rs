//! Lemnos's slot files on the data partition, as Gaia writes them:
//!
//! - `slots/<id>.json`: the slot's manifest, schema 3 (the bundle's `slot.json`
//!   form). The reader takes `schema`, `id`, `image.size`, `image.sha256`, the four
//!   `verity.*` values (checked by [`crate::Verity`]) and `boot.files` (path under
//!   `boot/` to sha256). Other fields are not read.
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
use crate::verity::Verity;

/// The only schema this reader takes.
const SCHEMA: u64 = 3;

/// The manifest as the reader takes it: the verity values are checked by
/// [`parse_manifest`], which turns them into a [`Verity`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct Raw {
    schema: u64,
    id: String,
    image: ImageField,
    verity: RawVerity,
    #[serde(default)]
    boot: BootField,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct RawVerity {
    root_hash: String,
    salt: String,
    data_blocks: u64,
    hash_offset: u64,
}

/// A checked manifest: the verity values are a [`Verity`] that fits the slot's
/// `image.size`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub(crate) id: String,
    pub(crate) image: ImageField,
    pub(crate) verity: Verity,
    pub(crate) boot: BootField,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct ImageField {
    pub(crate) size: u64,
    pub(crate) sha256: String,
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

/// Reads the manifest text of slot `id` and checks what the reader relies on:
/// schema 3, the slot's name, the digests, the verity values against the layout,
/// and the length of the slot file against them.
pub(crate) fn parse_manifest(text: &str, id: &str) -> Result<Manifest, ImageError> {
    let raw: Raw = serde_json::from_str(text)
        .map_err(|error| bad(format!("slot {id:?}'s manifest: {error}")))?;
    if raw.schema != SCHEMA {
        return Err(bad(format!(
            "slot {id:?}'s manifest is schema {}; this reader takes schema {SCHEMA} only",
            raw.schema
        )));
    }
    if raw.id != id {
        return Err(bad(format!(
            "slot {id:?}'s manifest says it is slot {:?}",
            raw.id
        )));
    }
    if !is_sha256(&raw.image.sha256) {
        return Err(bad(format!(
            "slot {id:?}'s image.sha256 is not a SHA-256 digest"
        )));
    }
    let verity = Verity::from_manifest(
        &raw.verity.root_hash,
        &raw.verity.salt,
        raw.verity.data_blocks,
        raw.verity.hash_offset,
    )
    .map_err(|error| bad(format!("slot {id:?}: {error}")))?;
    verity
        .check_size(raw.image.size)
        .map_err(|error| bad(format!("slot {id:?}: {error}")))?;
    for (path, sha) in &raw.boot.files {
        names::split_rel(path)?;
        if !is_sha256(sha) {
            return Err(bad(format!(
                "slot {id:?}: boot file {path:?} has no SHA-256"
            )));
        }
    }
    Ok(Manifest {
        id: raw.id,
        image: raw.image,
        verity,
        boot: raw.boot,
    })
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
    const ROOT: &str = "ab00000000000000000000000000000000000000000000000000000000000000";

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

    /// A schema 3 manifest of slot `alpha`: four data blocks, one tree block.
    fn manifest(root: &str, salt: &str, data_blocks: u64, hash_offset: u64, size: u64) -> String {
        format!(
            r#"{{"schema":3,"id":"alpha","product":"example","version":"1","board":"example-board",
                "image":{{"size":{size},"sha256":"{DIGEST}"}},
                "verity":{{"root_hash":"{root}","salt":"{salt}","data_blocks":{data_blocks},"hash_offset":{hash_offset}}},
                "boot":{{"files":{{"Image":"{DIGEST}"}}}}}}"#
        )
    }

    #[test]
    fn a_manifest_must_name_its_slot_and_hash_its_parts() {
        let good = manifest(ROOT, "5a5a", 4, 4 * 4096, 5 * 4096);
        let parsed = parse_manifest(&good, "alpha").expect("good");
        assert_eq!(parsed.verity.salt, vec![0x5a, 0x5a]);
        assert_eq!(parsed.verity.data_blocks, 4);
        assert_eq!(parsed.verity.hash_offset, 4 * 4096);
        assert!(parse_manifest(&good, "beta").is_err());
        let short = good.replace(DIGEST, "abc");
        assert!(parse_manifest(&short, "alpha").is_err());
        assert!(parse_manifest("{", "alpha").is_err());
    }

    #[test]
    fn schema_2_is_refused_by_name() {
        let text =
            manifest(ROOT, "", 4, 4 * 4096, 5 * 4096).replace("\"schema\":3", "\"schema\":2");
        let error = parse_manifest(&text, "alpha").expect_err("refused");
        assert!(error.to_string().contains("schema 2"), "{error}");
    }

    #[test]
    fn each_verity_value_that_does_not_fit_is_refused() {
        // A wrong salt, a wrong block count and a wrong offset each name their own fault.
        let bad_salt = manifest(ROOT, "xyz1", 4, 4 * 4096, 5 * 4096);
        assert!(
            parse_manifest(&bad_salt, "alpha")
                .unwrap_err()
                .to_string()
                .contains("salt")
        );
        let bad_offset = manifest(ROOT, "", 4, 3 * 4096, 5 * 4096);
        assert!(
            parse_manifest(&bad_offset, "alpha")
                .unwrap_err()
                .to_string()
                .contains("hash_offset")
        );
        // Blocks one too few, offset made to fit: the size rule names the tree.
        let bad_blocks = manifest(ROOT, "", 3, 3 * 4096, 5 * 4096);
        assert!(
            parse_manifest(&bad_blocks, "alpha")
                .unwrap_err()
                .to_string()
                .contains("hash tree")
        );
        // A tree cut short: image.size one block less than data plus tree.
        let cut = manifest(ROOT, "", 4, 4 * 4096, 4 * 4096);
        assert!(
            parse_manifest(&cut, "alpha")
                .unwrap_err()
                .to_string()
                .contains("hash tree")
        );
    }
}
