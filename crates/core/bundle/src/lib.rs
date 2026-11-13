//! Slot bundles: one slot of a first-flash image, made into the tar a board's
//! slot writer takes, and read back the way that writer reads it.
//!
//! A bundle is a plain tar holding `slot.json` (schema 2), `slot.sig`, the
//! slot's `rootfs.erofs` (the EROFS image with its dm-verity tree appended),
//! and the `boot/` files that go with it. `slot.sig` is the raw 64-byte
//! ed25519 signature over the exact bytes of `slot.json`. The format is
//! Lemnos's: its `system.slots` install takes exactly this.
//!
//! [`write_bundle`] makes one from a [`SlotSource`]; [`inspect_bundle`] reads
//! one back with the board's checks; [`verify_slot`] checks a signature over
//! `slot.json` on its own.
//!
//! The source is a seam. The real one reads the slot out of a first-flash
//! `.img.xz` (a streaming xz decoder and a read-only ext4 and FAT reader
//! over a bounded window). That reader is blocked: Gaia `next` cannot yet
//! produce such an image (F7 puts the slot file on the data partition, F8
//! wires the OS boot files into the boot partition), so there is no image to
//! read and no Gaia output to match. Every other part of this crate runs
//! against a [`SlotSource`] test double.
//!
//! Open: the verity salt. Gaia's `verity.table` gives the salt, and the root
//! hash is recomputed from the data with it; the hash is checked against the
//! table, but not the stored tree block by block. dm-verity checks the tree at
//! boot. The tree's layout (the root block's place) is not yet confirmed
//! against `veritysetup` output, since no `veritysetup` is installed here.

mod error;
mod inspect;
mod manifest;
mod sign;
mod source;
mod verity;
mod write;

pub use error::BundleError;
pub use inspect::{Inspected, inspect_bundle};
pub use manifest::{
    BootEntry, CMDLINE, ImageEntry, SCHEMA, SlotManifest, SlotMeta, VerityEntry,
    validate_boot_path, validate_id,
};
pub use sign::{SIG_LEN, sign_slot, verify_slot};
pub use source::{BootFile, SlotSource};
pub use verity::{BLOCK, VerityHasher, VerityTable, root_hash};
pub use write::{BOOT_DIR, ROOTFS, SLOT_JSON, SLOT_SIG, write_bundle};
