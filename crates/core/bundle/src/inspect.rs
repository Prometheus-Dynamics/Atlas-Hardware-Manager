//! Reading a bundle back: the checks a board's slot writer makes, streamed.
//!
//! The members come in the writer's order: `slot.json`, `slot.sig`,
//! `rootfs.erofs`, then `boot/` files. `slot.json` is checked against the
//! trusted keys before any of its fields is believed; then the rootfs and each
//! boot file are hashed against the manifest. A member the manifest does not
//! list, a link, a directory, or a missing file is refused.

use std::collections::BTreeSet;
use std::io::Read;

use atlas_release::PublicKey;
use sha2::{Digest, Sha256};

use crate::BundleError;
use crate::manifest::{SlotManifest, validate_boot_path};
use crate::sign::{SIG_LEN, verify_slot};
use crate::write::{BOOT_DIR, CHUNK, ROOTFS, SLOT_JSON, SLOT_SIG, read_chunk, to_hex};

/// `slot.json` is a small document: a larger member is refused before it is read.
const MAX_MANIFEST: u64 = 1024 * 1024;

/// A bundle that passed every check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspected {
    pub manifest: SlotManifest,
    /// The index in the keys given of the key that signed `slot.json`.
    pub signer: usize,
}

/// Reads a whole bundle from `reader` and checks it against `keys`.
pub fn inspect_bundle<R: Read>(reader: R, keys: &[PublicKey]) -> Result<Inspected, BundleError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|error| malformed(format!("not a tar: {error}")))?;

    let mut json: Vec<u8> = Vec::new();
    let mut manifest: Option<SlotManifest> = None;
    let mut signer = 0;
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

        match index {
            0 => {
                if name != SLOT_JSON {
                    return Err(malformed("slot.json must be the first member".into()));
                }
                if size > MAX_MANIFEST {
                    return Err(malformed(format!("slot.json is {size} bytes")));
                }
                read_exact_len(&mut entry, &mut json, size, SLOT_JSON)?;
            }
            1 => {
                if name != SLOT_SIG {
                    return Err(malformed("slot.sig must be the second member".into()));
                }
                let mut signature = Vec::new();
                if size != SIG_LEN as u64 {
                    return Err(malformed(format!(
                        "slot.sig is {size} bytes, not {SIG_LEN}"
                    )));
                }
                read_exact_len(&mut entry, &mut signature, size, SLOT_SIG)?;
                signer = verify_slot(&json, &signature, keys)?;
                let parsed: SlotManifest = serde_json::from_slice(&json)
                    .map_err(|error| malformed(format!("slot.json: {error}")))?;
                parsed.validate()?;
                manifest = Some(parsed);
            }
            _ => {
                let Some(manifest) = manifest.as_ref() else {
                    return Err(malformed("slot.sig is missing".into()));
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
                    check_stream(&mut entry, size, &mut buf, &name, size, expected)?;
                } else {
                    return Err(malformed(format!("{name} is not part of a slot bundle")));
                }
            }
        }
    }

    let Some(manifest) = manifest else {
        return Err(malformed("the bundle has no slot.sig".into()));
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
        return Err(malformed(format!("boot/{missing} is missing")));
    }
    Ok(Inspected { manifest, signer })
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
