//! The bundle of one slot of a first-flash image, rebuilt from what the image keeps.
//!
//! A slot installed on the image's data partition keeps the bundle's `slot.json` as
//! `slots/<id>.json`, byte for byte, and, when that bundle was signed, its `slot.sig` record as
//! `slots/<id>.sig`, unchanged (Lemnos docs/slots.md, "The kept signature"). A tar of those two
//! first, then `rootfs.erofs` and `boot/`, each copied unchanged, verifies as the bundle did. That is
//! the bundle made here: nothing is signed or written on the computer, so no key is needed, and an
//! unsigned slot gives an unsigned bundle.
//!
//! What is copied, in Lemnos's member order:
//!
//! - `slot.json`: the image's manifest, exactly as the image holds it;
//! - `slot.sig`: the kept record, when there is one;
//! - `rootfs.erofs`: the slot file, its length and SHA-256 checked as it streams against the
//!   manifest's `image.size` and `image.sha256`;
//! - `boot/`: the directory entry, as Lemnos writes it;
//! - `boot/<path>`: the boot files the manifest names (`boot.files`), in its order, each checked as it streams
//!   against its digest. The image's boot partition also holds `slot.json` and any file beside
//!   them; the manifest does not name those, and they are not members.
//!
//! The framing is Lemnos's own (see `TarOut`), so the bundle made here is byte for byte the one
//! `lemnos-ctl bundle` made for the image, when the image holds what the build wrote.
//!
//! The board checks the signature against the manifest's bytes; nothing here judges it.

use std::io::Write;
use std::path::Path;

use atlas_image::Options;

use crate::BundleError;
use crate::manifest::SlotManifest;
use crate::write::{BOOT_DIR, ROOTFS, SLOT_JSON, SLOT_SIG, TarOut};

/// What a bundle made from an image carries.
#[derive(Clone, Debug)]
pub struct ImageBundle {
    /// The image's manifest, as read (the bytes are the bundle's `slot.json`).
    pub manifest: SlotManifest,
    /// Whether the image kept a signature, so the bundle has a `slot.sig`.
    pub signed: bool,
}

/// Writes the bundle of slot `slot_id` of the first-flash image at `image` (`.img.xz` or `.img`) to
/// `out`, from the image's own manifest and kept signature (see the module docs).
///
/// The image is read with its scratch directory under `scratch_parent`; the scratch is removed
/// before this returns, on success or failure. On failure `out` holds a partial tar that must be
/// discarded.
pub fn bundle_from_image<W: Write>(
    image: &Path,
    scratch_parent: &Path,
    slot_id: &str,
    out: W,
) -> Result<ImageBundle, BundleError> {
    let image = atlas_image::open(image, scratch_parent, &[slot_id], &Options::default())?;
    let slot = image.slot(slot_id)?;
    let json = slot.manifest_json().as_bytes();
    let manifest = SlotManifest::parse(json)?;
    let signature = slot.signature();

    let mut tar = TarOut::new(out);
    tar.bytes(SLOT_JSON, json)?;
    if let Some(signature) = signature {
        tar.bytes(SLOT_SIG, signature)?;
    }
    let mut rootfs = slot.rootfs()?;
    tar.stream(ROOTFS, slot.image_size(), slot.image_sha256(), &mut *rootfs)
        .map_err(|error| match error {
            // The image reader checks the slot file's length at open, not its digest: a file that is not
            // its manifest's is the image's fault, said as such.
            BundleError::Changed {
                expected, actual, ..
            } => BundleError::ImageDigest { expected, actual },
            other => other,
        })?;
    tar.dir(BOOT_DIR)?;
    // In the order slot.json lists them, as Lemnos writes them (role order), not sorted by path: Lemnos's
    // reader keeps that order, where the manifest above keeps a map.
    let order = lemnos_slot_store::manifest::Manifest::parse(
        std::str::from_utf8(json)
            .map_err(|error| BundleError::Malformed(format!("slot.json: {error}")))?,
    )
    .map_err(BundleError::Malformed)?
    .boot_files;
    let mut kept = slot.manifest_boot_files();
    for (path, _) in &order {
        let Some(at) = kept.iter().position(|entry| &entry.path == path) else {
            continue;
        };
        let entry = kept.swap_remove(at);
        let mut reader = slot.boot_file(&entry.path)?;
        let name = format!("{BOOT_DIR}{}", entry.path);
        tar.stream(&name, entry.size, &entry.sha256, &mut *reader)?;
    }
    tar.finish()?;
    Ok(ImageBundle {
        manifest,
        signed: signature.is_some(),
    })
}
