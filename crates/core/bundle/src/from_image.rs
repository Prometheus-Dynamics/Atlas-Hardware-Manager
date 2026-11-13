//! The bundle made from a first-flash image: [`ImageSource`] is `atlas-image`'s
//! slot seen as a [`SlotSource`], and [`bundle_from_image`] is the one call
//! that opens the image, takes a slot and writes its bundle.
//!
//! What the image gives and what it does not. The image's slot has the slot
//! file (`rootfs`), its boot files, and the manifest's `verity.root_hash`,
//! `image.size` and `image.sha256`. It does not have the verity salt or the
//! data block count, which a [`SlotSource::verity_table`] needs to recompute
//! the root hash. So [`ImageSource::verity_table`] returns
//! [`BundleError::NoVerityTable`], and [`bundle_from_image`] stops there: no
//! value is invented. It waits on Lemnos's answer about which `verity.*` keys
//! `slot.json` carries (or on `atlas-image` reading them). The image's
//! `slot.json` is not a boot file of the bundle: the bundle writes its own.

use std::io::{Read, Write};
use std::path::Path;

use atlas_image::{Options, Slot};
use sha2::{Digest, Sha256};

use crate::BundleError;
use crate::manifest::{SlotManifest, SlotMeta};
use crate::source::{BootFile, SlotSource};
use crate::write::{CHUNK, SLOT_JSON, read_chunk, to_hex, write_bundle};

/// One slot of a first-flash image as a bundle source.
pub struct ImageSource<'a> {
    slot: Slot<'a>,
}

impl std::fmt::Debug for ImageSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageSource")
            .field("slot", &self.slot)
            .finish()
    }
}

impl<'a> ImageSource<'a> {
    pub fn new(slot: Slot<'a>) -> Self {
        Self { slot }
    }

    /// The slot id the image holds.
    pub fn id(&self) -> &str {
        self.slot.id()
    }

    /// Streams the slot file and checks its length and SHA-256 against the
    /// image manifest's `image.size` and `image.sha256`. The image reader
    /// checks the length at open but not the digest, so this reads the file.
    pub fn check_image_digest(&self) -> Result<(), BundleError> {
        let mut reader = self.slot.rootfs()?;
        check_stream_digest(
            &mut *reader,
            self.slot.image_size(),
            self.slot.image_sha256(),
        )
    }
}

impl SlotSource for ImageSource<'_> {
    fn verity_table(&self) -> Result<String, BundleError> {
        Err(BundleError::NoVerityTable(format!(
            "slot {:?} of the image: its manifest gives the verity root hash {} but not the salt or the data block count a verity table needs, so no bundle is made from it yet (waits on Lemnos's answer about the verity.* keys slot.json carries)",
            self.slot.id(),
            self.slot.root_hash()
        )))
    }

    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError> {
        Ok(self.slot.rootfs()?)
    }

    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError> {
        Ok(self
            .slot
            .boot_files()
            .into_iter()
            .filter(|entry| entry.path != SLOT_JSON)
            .map(|entry| BootFile {
                path: entry.path,
                size: entry.size,
            })
            .collect())
    }

    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError> {
        Ok(self.slot.boot_file(path)?)
    }
}

/// Makes the bundle of slot `slot_id` of the first-flash image at `image`
/// (`.img.xz` or `.img`), signed with `signing_key`, written to `out`.
///
/// The image is read with its scratch directory under `scratch_parent`; the
/// scratch is removed before this returns, on success or failure. Order: open
/// the image, take the slot (by id: the reader has no listing of slots), check
/// the slot file's digest against its manifest, then write the bundle. Today
/// the last step returns [`BundleError::NoVerityTable`]; see the module docs.
pub fn bundle_from_image<W: Write>(
    image: &Path,
    scratch_parent: &Path,
    slot_id: &str,
    meta: &SlotMeta,
    signing_key: &[u8; 32],
    out: W,
) -> Result<SlotManifest, BundleError> {
    let image = atlas_image::open(image, scratch_parent, &[slot_id], &Options::default())?;
    let source = ImageSource::new(image.slot(slot_id)?);
    source.check_image_digest()?;
    write_bundle(&source, slot_id, meta, signing_key, out)
}

/// Streams `reader` and checks it is `size` bytes with SHA-256 `sha256`
/// (lowercase hex). The error names both.
pub fn check_stream_digest(
    reader: &mut dyn Read,
    size: u64,
    sha256: &str,
) -> Result<(), BundleError> {
    let mut sha = Sha256::new();
    let mut count = 0u64;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let read = read_chunk(reader, &mut buf, "the slot file")?;
        if read == 0 {
            break;
        }
        sha.update(&buf[..read]);
        count += read as u64;
    }
    let actual = to_hex(&sha.finalize());
    if count != size || actual != sha256 {
        return Err(BundleError::ImageDigest {
            expected: format!("{size} bytes, sha256 {sha256}"),
            actual: format!("{count} bytes, sha256 {actual}"),
        });
    }
    Ok(())
}
