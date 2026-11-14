//! Reading a bundle back: the checks a board's slot writer makes, streamed.
//!
//! Member order and set, as Lemnos states them (I5g2 and I5i4):
//!
//! - `slot.json` is first;
//! - `slot.sig` is second when present (an unsigned bundle is legal);
//! - then `rootfs.erofs` and `boot/...` files in any order, each at most once and each listed in
//!   `slot.json`;
//! - anything else is refused by name, at its header, before its bytes are read;
//! - a listed member that is missing is refused at the end.
//!
//! `slot.sig` is checked against the keys given (see [`crate::verify_slot`]) before `slot.json` is
//! believed. Every member is checked against its manifest digest. `boot/cmdline.txt` must carry the
//! word for each boot source the board states ([`BootRule`]).

use std::collections::BTreeSet;
use std::io::Read;

use atlas_release::PublicKey;
use sha2::{Digest, Sha256};

use crate::BundleError;
use crate::boot::BootRule;
use crate::manifest::{CMDLINE, SlotManifest, validate_boot_path};
use crate::sign::{RECORD_LEN, Signature, verify_slot};
use crate::write::{BOOT_DIR, CHUNK, ROOTFS, SLOT_JSON, SLOT_SIG, read_chunk, to_hex};

/// `slot.json` is a small document: a larger member is refused before it is read.
const MAX_MANIFEST: u64 = 1024 * 1024;
/// `boot/cmdline.txt` is one line: a larger member is refused before it is read.
const MAX_CMDLINE: u64 = 64 * 1024;

/// A bundle that passed every check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspected {
    pub manifest: SlotManifest,
    /// What the signature says. `Unsigned` is a legal bundle.
    pub signature: Signature,
    /// The board the slot was built for (`slot.json`'s `board`).
    pub board: String,
    /// The product the slot is of (`slot.json`'s `product`).
    pub product: String,
    /// The OS version the slot carries (`slot.json`'s `version`).
    pub version: String,
}

/// Refuses a slot built for another board. `bundle_board` is the bundle's
/// `board`, `target` the board the installer would push to. Both names are in
/// the error. Call this before anything is pushed.
pub fn check_board(bundle_board: &str, target: &str) -> Result<(), BundleError> {
    if bundle_board == target {
        Ok(())
    } else {
        Err(BundleError::BoardMismatch {
            bundle: bundle_board.to_string(),
            target: target.to_string(),
        })
    }
}

/// Reads a whole bundle from `reader`, checks its signature against `keys` and its command line
/// against `boot`.
pub fn inspect_bundle<R: Read>(
    reader: R,
    keys: &[PublicKey],
    boot: &BootRule,
) -> Result<Inspected, BundleError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|error| malformed(format!("not a tar: {error}")))?;

    let mut json: Vec<u8> = Vec::new();
    let mut manifest: Option<SlotManifest> = None;
    let mut signature = Signature::Unsigned;
    let mut rootfs_done = false;
    let mut boot_done = BTreeSet::new();
    let mut buf = vec![0u8; CHUNK];

    for (index, entry) in entries.enumerate() {
        let mut entry = entry.map_err(|error| malformed(format!("bad tar entry: {error}")))?;
        let name = entry
            .path()
            .map_err(|error| malformed(format!("bad member name: {error}")))?
            .to_string_lossy()
            .into_owned();
        if !matches!(
            entry.header().entry_type(),
            tar::EntryType::Regular | tar::EntryType::Continuous
        ) {
            return Err(malformed(format!("{name} is not a regular file")));
        }
        let size = entry.size();

        if index == 0 {
            if name != SLOT_JSON {
                return Err(malformed(format!(
                    "{name} is the first member: slot.json must be the first member"
                )));
            }
            if size > MAX_MANIFEST {
                return Err(malformed(format!("slot.json is {size} bytes")));
            }
            read_exact_len(&mut entry, &mut json, size, SLOT_JSON)?;
            continue;
        }

        if index == 1 && name == SLOT_SIG {
            if size != RECORD_LEN as u64 {
                return Err(malformed(format!(
                    "slot.sig is {size} bytes, not the {RECORD_LEN}-byte signature record"
                )));
            }
            let mut record = Vec::new();
            read_exact_len(&mut entry, &mut record, size, SLOT_SIG)?;
            signature = verify_slot(&json, &record, keys)?;
            manifest = Some(SlotManifest::parse(&json)?);
            continue;
        }

        if name == SLOT_SIG {
            return Err(malformed(
                "slot.sig is out of order: it must be the second member, right after slot.json"
                    .into(),
            ));
        }
        if name == SLOT_JSON {
            return Err(malformed(
                "slot.json is listed twice: it must be the first member, once".into(),
            ));
        }
        if manifest.is_none() {
            // Unsigned (no slot.sig at index 1): the manifest is first believed here.
            manifest = Some(SlotManifest::parse(&json)?);
        }
        let Some(manifest) = manifest.as_ref() else {
            return Err(malformed("slot.json was not read".into()));
        };

        if name == ROOTFS {
            if rootfs_done {
                return Err(malformed(format!("{ROOTFS} is listed twice")));
            }
            rootfs_done = true;
            check_stream(
                &mut entry,
                size,
                &mut buf,
                ROOTFS,
                manifest.image.size,
                &manifest.image.sha256,
            )?;
        } else if let Some(path) = name.strip_prefix(BOOT_DIR) {
            validate_boot_path(path)?;
            let Some(expected) = manifest.boot.files.get(path) else {
                return Err(malformed(format!("{name} is not in slot.json")));
            };
            if !boot_done.insert(path.to_string()) {
                return Err(malformed(format!("{name} is listed twice")));
            }
            if path == CMDLINE {
                if size > MAX_CMDLINE {
                    return Err(malformed(format!("{name} is {size} bytes")));
                }
                let mut cmdline = Vec::new();
                read_exact_len(&mut entry, &mut cmdline, size, &name)?;
                check_digest(&cmdline, &name, size, expected)?;
                boot.check_cmdline(&cmdline)?;
            } else {
                check_stream(&mut entry, size, &mut buf, &name, size, expected)?;
            }
        } else {
            return Err(malformed(format!("{name} is not part of a slot bundle")));
        }
    }

    let Some(manifest) = manifest else {
        return Err(malformed(format!(
            "the bundle has only {SLOT_JSON}: no {ROOTFS}"
        )));
    };
    if !rootfs_done {
        return Err(malformed(format!("the bundle has no {ROOTFS}")));
    }
    if let Some(missing) = manifest
        .boot
        .files
        .keys()
        .find(|path| !boot_done.contains(*path))
    {
        return Err(malformed(format!(
            "boot/{missing} is listed in slot.json but missing"
        )));
    }
    Ok(Inspected {
        board: manifest.board.clone(),
        product: manifest.product.clone(),
        version: manifest.version.clone(),
        manifest,
        signature,
    })
}

fn malformed(message: String) -> BundleError {
    BundleError::Malformed(message)
}

/// Reads exactly `size` bytes of a member into `out`.
fn read_exact_len<R: Read>(
    entry: &mut R,
    out: &mut Vec<u8>,
    size: u64,
    what: &str,
) -> Result<(), BundleError> {
    entry
        .take(size)
        .read_to_end(out)
        .map_err(|error| BundleError::io(what, error))?;
    if out.len() as u64 != size {
        return Err(malformed(format!("{what} is cut short")));
    }
    Ok(())
}

/// Checks a member's bytes against its length and SHA-256 in the manifest.
fn check_digest(
    bytes: &[u8],
    name: &str,
    size: u64,
    expected_sha: &str,
) -> Result<(), BundleError> {
    let actual = to_hex(&Sha256::digest(bytes));
    if bytes.len() as u64 != size || actual != expected_sha {
        return Err(malformed(format!(
            "{name} doesn't match slot.json: sha256 {actual}, expected {expected_sha}"
        )));
    }
    Ok(())
}

/// Hashes a member's stream and checks its length and digest.
fn check_stream<R: Read>(
    entry: &mut R,
    size: u64,
    buf: &mut [u8],
    name: &str,
    expected_size: u64,
    expected_sha: &str,
) -> Result<(), BundleError> {
    if size != expected_size {
        return Err(malformed(format!(
            "{name} is {size} bytes, slot.json says {expected_size}"
        )));
    }
    let mut sha = Sha256::new();
    let mut count = 0u64;
    let mut limited = entry.take(size);
    loop {
        let read = read_chunk(&mut limited, buf, name)?;
        if read == 0 {
            break;
        }
        sha.update(&buf[..read]);
        count += read as u64;
    }
    let actual = to_hex(&sha.finalize());
    if count != size || actual != expected_sha {
        return Err(malformed(format!(
            "{name} doesn't match slot.json: sha256 {actual}, expected {expected_sha}"
        )));
    }
    Ok(())
}
