//! `slot.json`, schema 3: the manifest every slot bundle carries.

use std::collections::BTreeMap;

use atlas_image::Verity;
use serde::{Deserialize, Serialize};

use crate::BundleError;

/// The manifest's schema. Schema 2 (with `family`) is refused by name.
pub const SCHEMA: u32 = 3;
/// The boot file every slot must have.
pub const CMDLINE: &str = "cmdline.txt";
/// The five boot files every slot carries, at these fixed names, in the order Lemnos's bays take
/// them (`ROLES` in `drivers/slot-store/src/bays.rs` at Lemnos 4e871efd, I5i8): each is listed in
/// `boot.files` with its SHA-256, and nothing else is listed. Lemnos's install refuses a bundle
/// that lacks one with `the bundle has no boot/<name>`, and checks each file's size against its
/// bay (the bays are the board's; a writer does not know them).
pub const BOOT_FILES: [&str; 5] = [
    "kernel8.img",
    "initramfs.cpio",
    "board.dtb",
    CMDLINE,
    "overlays/lemnos-board.dtbo",
];

/// The fields a slot is made with beside the image and boot files. `product`,
/// `version` and `board` are required; the rest are left out of the JSON when
/// absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotMeta {
    /// The product name, lowercase `[a-z0-9-]`.
    pub product: String,
    /// The OS version the slot carries.
    pub version: String,
    pub variant: Option<String>,
    /// The board the slot was built for: the board file's `[board] id`, the
    /// directory name under `devices/boards`, lowercase `[a-z0-9-]`.
    pub board: String,
    pub kernel: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotManifest {
    pub schema: u32,
    pub id: String,
    pub product: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub board: String,
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

/// `verity`: the four values of the slot's dm-verity tree. The layout they
/// describe (format 1, SHA-256, 4 KiB blocks, no superblock, the tree appended
/// after the data) is fixed by convention and is not written here; see
/// `atlas-image`'s `verity` module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerityEntry {
    /// The root hash, 64 lowercase hex digits.
    pub root_hash: String,
    /// The salt: required, 64 lowercase hex digits (32 bytes). `None` only when a
    /// manifest read from a bundle lacks the key; the checks refuse it.
    pub salt: Option<String>,
    /// The data's length in 4096-byte blocks.
    pub data_blocks: u64,
    /// The byte offset of the tree: `data_blocks * 4096`.
    pub hash_offset: u64,
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
        verity: &Verity,
        boot: BTreeMap<String, String>,
    ) -> Self {
        Self {
            schema: SCHEMA,
            id: id.to_string(),
            product: meta.product.clone(),
            version: meta.version.clone(),
            variant: meta.variant.clone(),
            board: meta.board.clone(),
            kernel: meta.kernel.clone(),
            display_name: meta.display_name.clone(),
            image,
            verity: VerityEntry {
                root_hash: verity.root_hash.clone(),
                salt: Some(verity.salt_hex()),
                data_blocks: verity.data_blocks,
                hash_offset: verity.hash_offset,
            },
            boot: BootEntry { files: boot },
        }
    }

    /// Reads `slot.json`. Only schema 3 is taken. A schema 2 manifest, or one
    /// with a `family` key (schema 2's, which schema 3 does not have), is
    /// refused by name.
    pub fn parse(bytes: &[u8]) -> Result<Self, BundleError> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| BundleError::Malformed(format!("slot.json: {error}")))?;
        match value.get("schema").and_then(serde_json::Value::as_u64) {
            Some(found) if found == u64::from(SCHEMA) => {}
            Some(found) => {
                return Err(BundleError::Malformed(format!(
                    "slot.json schema {found} is not {SCHEMA}: this reader takes schema {SCHEMA} only"
                )));
            }
            None => {
                return Err(BundleError::Malformed(
                    "slot.json has no schema number".into(),
                ));
            }
        }
        if value.get("family").is_some() {
            return Err(BundleError::Malformed(
                "slot.json has a family key, which schema 3 does not have (the board is named by board)".into(),
            ));
        }
        let manifest: Self = serde_json::from_value(value)
            .map_err(|error| BundleError::Malformed(format!("slot.json: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
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
        validate_name("product", &self.product)?;
        validate_name("board", &self.board)?;
        validate_version(&self.version)?;
        if !is_hex(&self.image.sha256, 64) {
            return Err(BundleError::Malformed(
                "image.sha256 is not a SHA-256 digest".into(),
            ));
        }
        // The verity values: the layout, the salt's hex, and the file's length as
        // the manifest gives it (the same checks the image reader and the board make).
        Verity::from_manifest(
            &self.verity.root_hash,
            self.verity.salt.as_deref(),
            self.verity.data_blocks,
            self.verity.hash_offset,
        )?
        .check_size(self.image.size)?;
        for (path, digest) in &self.boot.files {
            validate_boot_path(path)?;
            if !is_hex(digest, 64) {
                return Err(BundleError::Malformed(format!(
                    "boot file {path} has no SHA-256 digest"
                )));
            }
        }
        check_boot_set(self.boot.files.keys().map(String::as_str))?;
        Ok(())
    }
}

/// Checks that the boot files listed are exactly the five of [`BOOT_FILES`]. A missing one is
/// refused by name; so is a sixth. Called by the writer before it signs, and by the reader.
pub fn check_boot_set<'a>(
    listed: impl Iterator<Item = &'a str> + Clone,
) -> Result<(), BundleError> {
    for name in BOOT_FILES {
        if !listed.clone().any(|path| path == name) {
            return Err(BundleError::MissingBootFile(name.to_string()));
        }
    }
    for path in listed {
        if !BOOT_FILES.contains(&path) {
            // Lemnos I5i9: replace with the board's exact words when it lands (today Lemnos
            // accepts an extra entry; the bundle rule here refuses it by name).
            return Err(BundleError::ExtraBootFile(path.to_string()));
        }
    }
    Ok(())
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

/// A product or board name: 1 to 64 of `[a-z0-9-]`.
pub fn validate_name(field: &'static str, name: &str) -> Result<(), BundleError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then_some(()).ok_or_else(|| BundleError::InvalidName {
        field,
        value: name.to_string(),
    })
}

fn validate_version(version: &str) -> Result<(), BundleError> {
    let ok = !version.is_empty()
        && version.len() <= 128
        && version.bytes().all(|b| (0x20..0x7f).contains(&b));
    ok.then_some(())
        .ok_or_else(|| BundleError::InvalidVersion(version.to_string()))
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
