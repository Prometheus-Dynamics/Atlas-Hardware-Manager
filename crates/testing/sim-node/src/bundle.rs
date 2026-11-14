//! The slot bundle the simulated board reads, as Lemnos states it (I5g2 for the signature, I5i4 and
//! I5i5 for the command line). A plain tar. Members, in this order:
//!
//! 1. `slot.json` (schema 3), always first;
//! 2. `slot.sig`, second when present: the 108-byte signature record, byte-identical to the
//!    first-flash record (`LMNSSIG1` | algorithm 1 | 3 zero bytes | key id = SHA-256 of the public
//!    key | the 64-byte Ed25519 signature);
//! 3. `rootfs.erofs` and `boot/...` files, in any order, each at most once and each listed in
//!    `slot.json`.
//!
//! Anything else is refused by name at its header, before its bytes are read. A signed message is
//! [`slot_message`]: the tag `lemnos-slot-bundle\0`, a u32 LE version and length, and the exact
//! `slot.json` bytes. This module keeps its own copy of that construction, separate from
//! `atlas-bundle`'s, so the fixed vector checks two implementations against each other.
//!
//! [`verify`] is the install's check on a finished bundle. [`build`] and [`sign`] make one, for tests.

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
pub const CMDLINE: &str = "boot/cmdline.txt";
const BOOT_PREFIX: &str = "boot/";
/// The five boot files every slot carries, at these fixed names, in the order Lemnos's bays take
/// them (`ROLES`, `drivers/slot-store/src/bays.rs:60-92` at Lemnos 4e871efd; I5i8). Each is listed in
/// `boot.files`, and nothing else is. `atlas-bundle`'s `BOOT_FILES` is the same table; a test checks
/// the two agree.
pub const BOOT_FILES: [&str; 5] = [
    "kernel8.img",
    "initramfs.cpio",
    "board.dtb",
    "cmdline.txt",
    "overlays/lemnos-board.dtbo",
];
/// The bay role of each boot file, in [`BOOT_FILES`] order: the words Lemnos's bay refusals use
/// (`bays.rs:400`).
pub const BAY_ROLES: [&str; 5] = [
    "kernel",
    "initramfs",
    "device-tree",
    "cmdline",
    "board-overlay",
];
/// The first-flash record's length, and the member that holds it.
pub const RECORD_LEN: usize = 108;
const RECORD_MAGIC: &[u8; 8] = b"LMNSSIG1";
/// The domain tag at the start of the signed message, with its NUL.
pub const SLOT_TAG: &[u8; 19] = b"lemnos-slot-bundle\0";

/// The command line a slot's `boot/cmdline.txt` must carry: the board's `boot_args` prefix, and for
/// each entry of `boot_sources` (`(source, kernel path)`) the word `<boot_args>.src.<source>=<path>`.
/// The installer appends nothing, so a slot whose line lacks a word is refused.
// LOCK: Lemnos L2 (I5i4, I5i5 final). Today the board has no boot_args/boot_sources keys; this is the
// shape the L2 board file states. The default is one source, the boot mode 1 path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootRule {
    pub boot_args: String,
    pub sources: Vec<(String, String)>,
}

impl Default for BootRule {
    fn default() -> Self {
        Self {
            boot_args: "os".into(),
            sources: vec![(
                "rpi-boot-mode-1".into(),
                "/soc@107c000000/mmc@fff000".into(),
            )],
        }
    }
}

impl BootRule {
    /// The word a line carries for one source.
    #[must_use]
    pub fn word(&self, source: &str, path: &str) -> String {
        format!("{}.src.{source}={path}", self.boot_args)
    }
}

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
    /// The boot files the manifest lists, by path, with their SHA-256 digests.
    pub boot: BTreeMap<String, String>,
    /// The stored size of each of the five boot files, in [`BOOT_FILES`] order: what the board's
    /// bay for that role must hold (see `Slots::check_bays`).
    pub payloads: [u64; 5],
}

/// The first-flash record, as this board reads it.
struct Record {
    key_id: [u8; 32],
    signature: [u8; 64],
}

// LOCK: Lemnos L2 (I5g2). Today Lemnos reads a bare 64-byte `slot.sig`, a `slot.key` member, and
// signs the bare manifest bytes. At the lock move: this record (108 bytes, key id inside), this
// message, and the member order below become Lemnos's; the fixed vector in the tests checks them.
/// The message a signature covers: the tag, a u32 LE version (1), a u32 LE length, the exact
/// `slot.json` bytes. The only construction of it in this crate.
#[must_use]
pub fn slot_message(slot_json: &[u8]) -> Vec<u8> {
    let length = u32::try_from(slot_json.len()).unwrap_or(u32::MAX);
    let mut message = Vec::with_capacity(SLOT_TAG.len() + 8 + slot_json.len());
    message.extend_from_slice(SLOT_TAG);
    message.extend_from_slice(&1u32.to_le_bytes());
    message.extend_from_slice(&length.to_le_bytes());
    message.extend_from_slice(slot_json);
    message
}

/// Reads a 108-byte record. Every malformed form is `INVALID_ARGUMENT`, whatever the key list.
fn parse_record(bytes: &[u8]) -> Result<Record, Status> {
    if bytes.len() != RECORD_LEN {
        return Err(refuse(format!(
            "slot.sig is {} bytes, not the {RECORD_LEN}-byte signature record",
            bytes.len()
        )));
    }
    if &bytes[..8] != RECORD_MAGIC {
        return Err(refuse("slot.sig does not start with LMNSSIG1"));
    }
    if bytes[8] != 1 {
        return Err(refuse(format!(
            "slot.sig's algorithm is {}; this board knows 1 (Ed25519)",
            bytes[8]
        )));
    }
    if bytes[9..12] != [0, 0, 0] {
        return Err(refuse(
            "slot.sig's three bytes after the algorithm are not zero",
        ));
    }
    let mut key_id = [0u8; 32];
    key_id.copy_from_slice(&bytes[12..44]);
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&bytes[44..108]);
    Ok(Record { key_id, signature })
}

// LOCK: Lemnos L2 (I5g2). Today: no `slot.sig` record; a known key with a bad signature installs as
// untrusted. The rule below is the L2 shape.
/// The signature rule: a record whose key id is on the board's list must verify, or it is refused
/// (`NOT_PERMITTED`); a key id on no list is `Untrusted` (not checked, not refused); no record is
/// `Unsigned`. Whether an `Untrusted` or `Unsigned` bundle installs is the security switch's
/// (`Slots::check_signed`), not this function's.
fn signature_of(record: &[u8], slot_json: &[u8], trusted: &[TrustedKey]) -> Result<Signed, Status> {
    let record = parse_record(record)?;
    let Some(key) = trusted
        .iter()
        .find(|key| Sha256::digest(key.public).as_slice() == record.key_id)
    else {
        return Ok(Signed::Untrusted);
    };
    let signature = Signature::from_bytes(&record.signature);
    let verifies = VerifyingKey::from_bytes(&key.public)
        .is_ok_and(|public| public.verify(&slot_message(slot_json), &signature).is_ok());
    if verifies {
        Ok(Signed::Trusted {
            owner: key.owner.clone(),
        })
    } else {
        Err(Status::new(
            code::NOT_PERMITTED,
            format!(
                "slot.sig is signed by a key this board holds ({}), owned by {}, and it does not verify slot.json: refused, nothing is written",
                hex(&record.key_id),
                key.owner
            ),
        ))
    }
}

/// Lowercase hex of a SHA-256 digest.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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

/// What `slot.json` says, checked as far as the header rules need it.
struct Manifest {
    value: Value,
    listed: BTreeSet<String>,
}

/// The boot-file set of `slot.json`: exactly [`BOOT_FILES`]. A missing one is refused with the
/// board's words (`install.rs:756`); a sixth one with ours, until Lemnos I5i9 lands.
fn check_boot_set(listed: &BTreeSet<String>) -> Result<(), Status> {
    for name in BOOT_FILES {
        if !listed.contains(name) {
            return Err(refuse(format!("the bundle has no {BOOT_PREFIX}{name}")));
        }
    }
    for name in listed {
        if !BOOT_FILES.contains(&name.as_str()) {
            // Lemnos I5i9: replace with the board's exact words when it lands.
            return Err(refuse(format!(
                "boot.files lists {name}, which is not one of the five boot files (kernel8.img, initramfs.cpio, board.dtb, cmdline.txt, overlays/lemnos-board.dtbo): refused"
            )));
        }
    }
    Ok(())
}

fn read_manifest(bytes: &[u8], board: &str) -> Result<Manifest, Status> {
    let manifest: Value = serde_json::from_slice(bytes)
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
        return Err(Status::new(
            code::NOT_SUPPORTED,
            format!("this slot is built for board {slot_board}, and this is board {board}"),
        ));
    }
    if manifest.get("layers").is_some() {
        return Err(refuse(
            "layered slots are not supported: send a single-image bundle",
        ));
    }
    let id = text(&manifest, "id")?;
    if !valid_id(id) {
        return Err(refuse(format!("{id:?} is not a slot id")));
    }
    let listed = manifest
        .get("boot")
        .and_then(|boot| boot.get("files"))
        .and_then(Value::as_object)
        .ok_or_else(|| refuse("slot.json has no boot.files"))?
        .keys()
        .cloned()
        .collect();
    check_boot_set(&listed)?;
    Ok(Manifest {
        value: manifest,
        listed,
    })
}

/// Checks a finished bundle and says what it holds. Nothing is kept: the caller installs
/// the slot only when this returns `Ok`.
///
/// The member order, set and digests are checked as members arrive; a refused bundle is refused
/// before anything is installed.
///
/// # Errors
///
/// `INVALID_ARGUMENT` for a malformed bundle: a member out of order, a member outside the set, a
/// listed member missing, a hash or size mismatch, a link or an absolute or `..` path, a malformed
/// signature record, or a command line without a word the board states. `NOT_SUPPORTED` for a slot
/// built for another board. `NOT_PERMITTED` for a signature by a held key that does not verify.
pub fn verify(
    bundle: &[u8],
    trusted: &[TrustedKey],
    board: &str,
    boot: &BootRule,
) -> Result<Verified, Status> {
    let mut archive = tar::Archive::new(Cursor::new(bundle));
    let entries = archive
        .entries()
        .map_err(|error| refuse(format!("not a tar: {error}")))?;

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut count = 0usize;
    let mut json: Option<Vec<u8>> = None;
    let mut manifest: Option<Manifest> = None;
    let mut signed = Signed::Unsigned;
    let mut members: BTreeMap<String, Vec<u8>> = BTreeMap::new();

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

        // The order rule first: slot.json is the first member, slot.sig the second when present.
        if count == 0 && name != SLOT_JSON {
            return Err(refuse(format!(
                "{name} is the first member: slot.json must be the first member"
            )));
        }
        if count > 0 && name == SLOT_JSON {
            return Err(refuse(
                "slot.json is listed twice: it must be the first member, once",
            ));
        }
        if name == SLOT_SIG && count != 1 {
            return Err(refuse(
                "slot.sig is out of order: it must be the second member, right after slot.json",
            ));
        }
        // The set rule, at the header: a name outside it, or a boot file slot.json does not list.
        let known = name == SLOT_JSON
            || name == SLOT_SIG
            || name == ROOTFS
            || name
                .strip_prefix(BOOT_PREFIX)
                .is_some_and(|rest| !rest.is_empty());
        if !known {
            return Err(refuse(format!("{name} is not part of a slot bundle")));
        }
        if let Some(rest) = name.strip_prefix(BOOT_PREFIX) {
            let listed = manifest
                .as_ref()
                .is_some_and(|manifest| manifest.listed.contains(rest));
            if !listed {
                // The board's words (`install.rs:211`, Lemnos 4e871efd), at the member's header.
                return Err(refuse(format!(
                    "boot file {rest} isn't in slot.json: a file the manifest does not list is refused"
                )));
            }
        }
        if !seen.insert(name.clone()) {
            return Err(refuse(format!("{name} is in the bundle twice")));
        }
        count += 1;

        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| refuse(format!("{name} is cut short: {error}")))?;
        match name.as_str() {
            SLOT_JSON => {
                manifest = Some(read_manifest(&bytes, board)?);
                json = Some(bytes);
            }
            SLOT_SIG => {
                let slot_json = json.as_deref().unwrap_or_default();
                signed = signature_of(&bytes, slot_json, trusted)?;
            }
            _ => {
                members.insert(name, bytes);
            }
        }
    }

    let Some(manifest) = manifest else {
        return Err(refuse("the bundle has no slot.json"));
    };
    let value = &manifest.value;
    let id = text(value, "id")?.to_string();
    let size = value
        .get("image")
        .and_then(|image| image.get("size"))
        .and_then(Value::as_u64)
        .ok_or_else(|| refuse("slot.json has no image size"))?;
    let image = value
        .get("image")
        .ok_or_else(|| refuse("slot.json has no image"))?;
    let image_hash = text(image, "sha256")?;
    let verity_root = value
        .get("verity")
        .and_then(|verity| verity.get("root_hash"))
        .and_then(Value::as_str);
    if !is_hex64(image_hash) || verity_root.is_some_and(|root| !is_hex64(root)) {
        return Err(refuse("a hash in slot.json is not 64 hex digits"));
    }

    let rootfs = members
        .get(ROOTFS)
        .ok_or_else(|| refuse(format!("the bundle has no {ROOTFS}")))?;
    if rootfs.len() as u64 != size {
        return Err(refuse(format!(
            "{ROOTFS} is {} bytes, slot.json says {size}",
            rootfs.len()
        )));
    }
    if sha256_hex(rootfs) != image_hash.to_ascii_lowercase() {
        return Err(refuse(format!("{ROOTFS} does not match image.sha256")));
    }

    let boot_files = value
        .get("boot")
        .and_then(|boot| boot.get("files"))
        .and_then(Value::as_object)
        .ok_or_else(|| refuse("slot.json has no boot.files"))?;
    for (path, expected) in boot_files {
        let member = format!("{BOOT_PREFIX}{path}");
        // The board's words for a listed file that is not carried (`install.rs:683`).
        let Some(bytes) = members.get(&member) else {
            return Err(refuse(format!("the bundle has no boot file {path}")));
        };
        let expected = expected.as_str().unwrap_or_default();
        if sha256_hex(bytes) != expected.to_ascii_lowercase() {
            return Err(refuse(format!(
                "{member} doesn't match slot.json: its sha256 is wrong"
            )));
        }
    }

    // The command line: the complete line, with one word per boot source. A slot without a
    // cmdline.txt listed is refused by the same rule.
    // Every listed boot file is a member (checked above), so this is the listed cmdline.txt.
    let cmdline = members
        .get(CMDLINE)
        .ok_or_else(|| refuse("boot.files has no cmdline.txt"))?;
    let text_line = String::from_utf8_lossy(cmdline);
    let words: Vec<&str> = text_line.split_ascii_whitespace().collect();
    for (source, path) in &boot.sources {
        let word = boot.word(source, path);
        if !words.iter().any(|candidate| *candidate == word) {
            return Err(refuse(format!(
                "{CMDLINE} has no {word:?}: the builder writes the whole line for boot source {source}"
            )));
        }
    }

    let payloads = BOOT_FILES.map(|name| {
        members
            .get(&format!("{BOOT_PREFIX}{name}"))
            .map_or(0, |bytes| bytes.len() as u64)
    });
    let boot = boot_files
        .iter()
        .map(|(path, digest)| {
            (
                path.clone(),
                digest.as_str().unwrap_or_default().to_ascii_lowercase(),
            )
        })
        .collect();
    Ok(Verified {
        id,
        product: text(value, "product")?.to_string(),
        board: text(value, "board")?.to_string(),
        version: text(value, "version").unwrap_or_default().to_string(),
        variant: text(value, "variant").unwrap_or_default().to_string(),
        kernel: text(value, "kernel").unwrap_or_default().to_string(),
        display_name: text(value, "display_name").unwrap_or_default().to_string(),
        size,
        verity: verity_root.is_some(),
        signed,
        boot,
        payloads,
    })
}

/// A manifest for `rootfs` and `boot`, with every hash filled in. Its command line is the one
/// [`BootRule::default`] takes, unless the caller gives its own `cmdline.txt`.
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

/// Placeholder bytes for the four boot files other than the command line, in [`BOOT_FILES`] order.
const PLACEHOLDERS: [&[u8]; 4] = [
    b"kernel8.img placeholder\n",
    b"initramfs.cpio placeholder\n",
    b"board.dtb placeholder\n",
    b"lemnos-board.dtbo placeholder\n",
];

/// The five boot files of a slot with `cmdline` as its command line: placeholders for the other
/// four. A slot carries all five (Lemnos I5i8), so a test's bundle does too.
#[must_use]
pub fn boot_files(cmdline: &[u8]) -> [(&'static str, &[u8]); 5] {
    [
        (BOOT_FILES[0], PLACEHOLDERS[0]),
        (BOOT_FILES[1], PLACEHOLDERS[1]),
        (BOOT_FILES[2], PLACEHOLDERS[2]),
        (BOOT_FILES[3], cmdline),
        (BOOT_FILES[4], PLACEHOLDERS[3]),
    ]
}

/// The command line that carries the default rule's word.
#[must_use]
pub fn default_cmdline() -> Vec<u8> {
    let rule = BootRule::default();
    format!(
        "console=ttyS0 root=/dev/dm-0 {}\n",
        rule.word("rpi-boot-mode-1", "/soc@107c000000/mmc@fff000")
    )
    .into_bytes()
}

/// Makes a bundle: `slot.json` from `manifest`, `slot.sig` (the 108-byte record) when given, the
/// rootfs and the boot files. The members are given as they are, so a test can make a malformed
/// one.
///
/// # Panics
///
/// Never in practice: the tar is written to memory.
#[must_use]
pub fn build(
    manifest: &Value,
    record: Option<&[u8]>,
    rootfs: &[u8],
    boot: &[(&str, &[u8])],
) -> Vec<u8> {
    let mut parts: Vec<(String, Vec<u8>)> =
        vec![(SLOT_JSON.to_string(), manifest.to_string().into_bytes())];
    if let Some(record) = record {
        parts.push((SLOT_SIG.to_string(), record.to_vec()));
    }
    parts.push((ROOTFS.to_string(), rootfs.to_vec()));
    for (path, bytes) in boot {
        parts.push((format!("{BOOT_PREFIX}{path}"), bytes.to_vec()));
    }
    tar_of(&parts)
}

/// A tar of `parts`, in the order given. For tests that make a bundle the writer would not.
///
/// # Panics
///
/// Never in practice: the tar is written to memory.
#[must_use]
pub fn tar_of(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in parts {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, &bytes[..])
            .expect("a member in memory");
    }
    builder.into_inner().expect("a tar in memory")
}

/// The 108-byte `slot.sig` record of `manifest` signed by `key`: the key id is the SHA-256 of its
/// public key, the signature is over [`slot_message`] of the manifest's exact bytes.
#[must_use]
pub fn sign(manifest: &Value, key: &SigningKey) -> [u8; RECORD_LEN] {
    let slot_json = manifest.to_string().into_bytes();
    let signature = key.sign(&slot_message(&slot_json)).to_bytes();
    record_of(key, &signature)
}

/// A record of `key` with `signature` as its signature (the caller decides what it signed).
#[must_use]
pub fn record_of(key: &SigningKey, signature: &[u8; 64]) -> [u8; RECORD_LEN] {
    let mut record = [0u8; RECORD_LEN];
    record[..8].copy_from_slice(RECORD_MAGIC);
    record[8] = 1;
    record[12..44].copy_from_slice(&Sha256::digest(key.verifying_key().to_bytes()));
    record[44..108].copy_from_slice(signature);
    record
}

/// The key id of a signer: the SHA-256 of its public key, as 64 lowercase hex digits.
#[must_use]
pub fn key_id(key: &SigningKey) -> String {
    sha256_hex(&key.verifying_key().to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed vector of Lemnos I5g2, as atlas-bundle's test has it: seed 0x07 x32, the manifest
    /// `{"id":"vector"}`.
    #[test]
    fn this_board_reproduces_the_fixed_vector() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let json = br#"{"id":"vector"}"#;
        assert_eq!(
            hex(&slot_message(json)),
            "6c656d6e6f732d736c6f742d62756e646c6500010000000f0000007b226964223a22766563746f72227d"
        );
        let signature = key.sign(&slot_message(json)).to_bytes();
        let record = record_of(&key, &signature);
        assert_eq!(
            hex(&record),
            concat!(
                "4c4d4e535349473101000000",
                "fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889",
                "56be579db020703eb207da579036a89f0321fcd8ce2bac78a9aa939a58217e265ad27a7e1fc05829b0db05d37ac4d8dba35263c7a1b2644322b45ada4a101402"
            )
        );
    }
}
