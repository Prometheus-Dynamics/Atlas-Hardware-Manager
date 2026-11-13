//! Slot bundles: one slot of a first-flash image, made into the tar a board's
//! slot writer takes, and read back the way that writer reads it.
//!
//! A bundle is a plain tar holding `slot.json` (schema 3), `slot.sig`, the
//! slot's `rootfs.erofs` (the EROFS image with its dm-verity tree appended),
//! and the `boot/` files that go with it. `slot.sig` is the raw 64-byte
//! ed25519 signature over the exact bytes of `slot.json`. The format is
//! Lemnos's: its `system.slots` install takes exactly this.
//!
//! `slot.json` carries `product`, `version`, optional `variant`, `board` (the
//! board file's `[board] id`, the board the slot was built for; a bundle
//! without it, or for another board, is refused), `id`, `kernel`,
//! `display_name`, `image.*`, `verity.root_hash` and `boot.files`. There is no
//! `family` key. Schema 2 is refused by name.
//!
//! The main entry points:
//!
//! - [`bundle_from_image`]: open a first-flash image (through `atlas-image`),
//!   take a slot by id, write its bundle. Not complete yet: it stops with
//!   [`BundleError::NoVerityTable`] (see below).
//! - [`write_bundle`]: make a bundle from any [`SlotSource`].
//! - [`inspect_bundle`]: read a bundle back with the board's checks; then
//!   [`check_board`] before anything is pushed.
//! - [`verify_slot`]: check a signature over `slot.json` on its own.
//!
//! The source is a seam, [`SlotSource`]. Its real implementation is
//! [`ImageSource`], over `atlas-image`'s slot. Tests use a double.
//!
//! Open, and why the bundle from an image is not complete. The seam asks for
//! a verity table (Gaia's `verity.table` text: the salt, the data block count,
//! and the root hash) so the writer can check the slot file's tree against
//! it. An image's manifest gives only the root hash, so the table cannot be
//! built and [`ImageSource`] returns [`BundleError::NoVerityTable`]. Nothing is
//! invented. The seam change that would take the root hash and the data
//! length instead waits on Lemnos's answer about the `verity.*` keys. Until
//! then:
//!
//! - the rootfs and boot files of an image slot are read through the seam as
//!   they are (their digests are checked against the image's manifest by
//!   [`ImageSource::check_image_digest`]);
//! - the verity root hash is not recomputed; the board's dm-verity checks the
//!   tree at boot. The `veritysetup` cross-check on a real Gaia slot has not
//!   been run.
//!
//! Untested on board.

mod error;
mod from_image;
mod inspect;
mod manifest;
mod sign;
mod source;
mod verity;
mod write;

pub use error::BundleError;
pub use from_image::{ImageSource, bundle_from_image, check_stream_digest};
pub use inspect::{Inspected, check_board, inspect_bundle};
pub use manifest::{
    BootEntry, CMDLINE, ImageEntry, SCHEMA, SlotManifest, SlotMeta, VerityEntry,
    validate_boot_path, validate_id, validate_name,
};
pub use sign::{SIG_LEN, sign_slot, verify_slot};
pub use source::{BootFile, SlotSource};
pub use verity::{BLOCK, VerityHasher, VerityTable, root_hash};
pub use write::{BOOT_DIR, ROOTFS, SLOT_JSON, SLOT_SIG, write_bundle};
