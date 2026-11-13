//! The slot bundle Lemnos reads (I5): a plain tar with `slot.json`, `rootfs.erofs` (the
//! verity tree appended), `boot/...` files, and an optional `slot.sig`, the raw 64-byte
//! Ed25519 signature over the exact bytes of `slot.json`.
//!
//! [`verify`] is the install's check on a finished bundle. [`build`] and [`sign`] make one,
//! for tests and for the sim's own "Atlas made this bundle" fixtures.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::Component;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::slots::{Signed, Status, TrustedKey, code, valid_id};

/// The slot.json schema this board reads: `product`, `version`, optional `variant`, `board`, and the
/// rest as before. Schema 2 is refused.
pub const SCHEMA: u64 = 3;
pub const SLOT_JSON: &str = "slot.json";
pub const SLOT_SIG: &str = "slot.sig";
pub const ROOTFS: &str = "rootfs.erofs";
const BOOT_PREFIX: &str = "boot/";

/// The facts of a verified bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub id: String,
    /// The product the slot is for (`product`), lowercase `[a-z0-9-]`.
    pub product: String,
    /// The board the slot was built for (`board`): the board's id, the same as its `family` fact.
    pub board: String,
    pub version: String,
    pub variant: String,
    pub kernel: String,
    pub display_name: String,
    pub size: u64,
    pub verity: bool,
    pub signed: Signed,
}

/// Lowercase hex of a SHA-256 digest.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A product or board id: lowercase letters, digits and `-`, not empty.
fn valid_key(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn refuse(message: impl Into<String>) -> Status {
    Status::new(code::INVALID_ARGUMENT, message)
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, Status> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| refuse(format!("slot.json has no {key}")))
}

/// Checks a finished bundle and says what it holds. Nothing is kept: the caller installs
/// the slot only when this returns `Ok`.
///
/// # Errors
///
/// `INVALID_ARGUMENT` for a malformed bundle, a hash or size mismatch, a link or an
/// absolute or `..` path, an unknown member, or a `slot.sig` that is not 64 bytes.
pub fn verify(bundle: &[u8], trusted: &[TrustedKey], board: &str) -> Result<Verified, Status> {
    let mut members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut archive = tar::Archive::new(Cursor::new(bundle));
    let entries = archive
        .entries()
        .map_err(|error| refuse(format!("not a tar: {error}")))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| refuse(format!("a bad tar entry: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| refuse(format!("a bad path: {error}")))?
            .into_owned();
        if path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(refuse("a path is absolute or goes up with .."));
        }
        let header = entry.header().entry_type();
        if header.is_dir() {
            continue;
        }
        if !header.is_file() {
            return Err(refuse("only plain files are allowed: no links, no devices"));
        }
        let name = path.to_string_lossy().into_owned();
        let known = name == SLOT_JSON
            || name == SLOT_SIG
            || name == ROOTFS
            || name
                .strip_prefix(BOOT_PREFIX)
                .is_some_and(|rest| !rest.is_empty());
        if !known {
            return Err(refuse(format!("unknown member {name}")));
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| refuse(format!("{name} is cut short: {error}")))?;
        if members.insert(name.clone(), bytes).is_some() {
            return Err(refuse(format!("{name} is in the bundle twice")));
        }
    }

    let manifest_bytes = members
        .get(SLOT_JSON)
        .ok_or_else(|| refuse("the bundle has no slot.json"))?
        .clone();
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| refuse(format!("slot.json is not JSON: {error}")))?;
    match manifest.get("schema").and_then(Value::as_u64) {
        Some(SCHEMA) => {}
        Some(other) => {
            return Err(refuse(format!(
                "slot.json is schema {other}; this board takes schema {SCHEMA} (product, version, board)"
            )));
        }
        None => return Err(refuse("slot.json has no schema")),
    }
    let product = text(&manifest, "product")?;
    if !valid_key(product) {
        return Err(refuse(format!("{product:?} is not a product name")));
    }
    let slot_board = text(&manifest, "board")
        .map_err(|_| refuse("slot.json has no board: the slot is not for any board"))?;
    if slot_board != board {
        return Err(refuse(format!(
            "the slot is for board {slot_board:?}, and this board is {board:?}"
        )));
    }
    if manifest.get("layers").is_some() {
        return Err(refuse(
            "layered slots are not supported: send a single-image bundle",
        ));
    }
    let id = text(&manifest, "id")?.to_string();
    if !valid_id(&id) {
        return Err(refuse(format!("{id:?} is not a slot id")));
    }
    let image = manifest
        .get("image")
        .ok_or_else(|| refuse("slot.json has no image"))?;
    let size = image
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| refuse("slot.json has no image size"))?;
    let image_hash = text(image, "sha256")?;
    let verity_root = manifest
        .get("verity")
        .and_then(|verity| verity.get("root_hash"))
        .and_then(Value::as_str);
    if !is_hex64(image_hash) || verity_root.is_some_and(|root| !is_hex64(root)) {
        return Err(refuse("a hash in slot.json is not 64 hex digits"));
    }

    let rootfs = members
        .get(ROOTFS)
        .ok_or_else(|| refuse("the bundle has no rootfs.erofs"))?;
    if rootfs.len() as u64 != size {
        return Err(refuse(format!(
            "rootfs.erofs is {} bytes, slot.json says {size}",
            rootfs.len()
        )));
    }
    if sha256_hex(rootfs) != image_hash.to_ascii_lowercase() {
        return Err(refuse("rootfs.erofs does not match image.sha256"));
    }

    let boot_files = manifest
        .get("boot")
        .and_then(|boot| boot.get("files"))
        .and_then(Value::as_object)
        .ok_or_else(|| refuse("slot.json has no boot.files"))?;
    if !boot_files.contains_key("cmdline.txt") {
        return Err(refuse("boot.files has no cmdline.txt"));
    }
    let on_disk: BTreeSet<String> = members
        .keys()
        .filter_map(|name| name.strip_prefix(BOOT_PREFIX).map(str::to_string))
        .collect();
    let listed: BTreeSet<String> = boot_files.keys().cloned().collect();
    if on_disk != listed {
        return Err(refuse(
            "boot/ holds files boot.files does not list, or lacks some it lists",
        ));
    }
    for (path, expected) in boot_files {
        let bytes = &members[&format!("{BOOT_PREFIX}{path}")];
        let expected = expected.as_str().unwrap_or_default();
        if sha256_hex(bytes) != expected.to_ascii_lowercase() {
            return Err(refuse(format!("boot/{path} does not match its hash")));
        }
    }

    let signed = match members.get(SLOT_SIG) {
        None => Signed::Unsigned,
        Some(signature) => {
            let signature: [u8; 64] = signature
                .as_slice()
                .try_into()
                .map_err(|_| refuse("slot.sig is not 64 bytes"))?;
            let signature = Signature::from_bytes(&signature);
            trusted
                .iter()
                .find(|key| {
                    VerifyingKey::from_bytes(&key.public)
                        .is_ok_and(|public| public.verify(&manifest_bytes, &signature).is_ok())
                })
                .map_or(Signed::Untrusted, |key| Signed::Trusted {
                    owner: key.owner.clone(),
                })
        }
    };

    Ok(Verified {
        id,
        product: product.to_string(),
        board: slot_board.to_string(),
        version: text(&manifest, "version").unwrap_or_default().to_string(),
        variant: text(&manifest, "variant").unwrap_or_default().to_string(),
        kernel: text(&manifest, "kernel").unwrap_or_default().to_string(),
        display_name: text(&manifest, "display_name")
            .unwrap_or_default()
            .to_string(),
        size,
        verity: verity_root.is_some(),
        signed,
    })
}

/// A manifest for `rootfs` and `boot`, with every hash filled in.
#[must_use]
pub fn manifest(id: &str, version: &str, rootfs: &[u8], boot: &[(&str, &[u8])]) -> Value {
    let files: BTreeMap<String, String> = boot
        .iter()
        .map(|(path, bytes)| ((*path).to_string(), sha256_hex(bytes)))
        .collect();
    json!({
        "schema": SCHEMA,
        "id": id,
        "product": "sim",
        "board": "simulated",
        "version": version,
        "variant": "",
        "kernel": "",
        "display_name": format!("Simulated {version}"),
        "image": { "size": rootfs.len(), "sha256": sha256_hex(rootfs) },
        "verity": { "root_hash": sha256_hex(&[rootfs, b"verity"].concat()) },
        "boot": { "files": files },
    })
}

/// Makes a bundle: `slot.json` from `manifest`, the signature if there is one, the rootfs
/// and the boot files.
///
/// # Panics
///
/// Never in practice: the tar is written to memory.
#[must_use]
pub fn build(
    manifest: &Value,
    signature: Option<&[u8; 64]>,
    rootfs: &[u8],
    boot: &[(&str, &[u8])],
) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let manifest_bytes = manifest.to_string().into_bytes();
    append(&mut builder, SLOT_JSON, &manifest_bytes);
    if let Some(signature) = signature {
        append(&mut builder, SLOT_SIG, signature);
    }
    append(&mut builder, ROOTFS, rootfs);
    for (path, bytes) in boot {
        append(&mut builder, &format!("{BOOT_PREFIX}{path}"), bytes);
    }
    builder.into_inner().expect("a tar in memory")
}

fn append(builder: &mut tar::Builder<Vec<u8>>, name: &str, bytes: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, name, bytes)
        .expect("a member in memory");
}

/// The 64-byte signature over the exact bytes of a manifest.
#[must_use]
pub fn sign(manifest: &Value, key: &SigningKey) -> [u8; 64] {
    key.sign(manifest.to_string().as_bytes()).to_bytes()
}
