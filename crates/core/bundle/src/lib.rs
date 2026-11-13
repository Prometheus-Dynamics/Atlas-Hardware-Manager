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
//! `display_name`, `image.*`, `verity.*` (the root hash, the salt, the data
//! block count and the tree's offset) and `boot.files`. There is no `family`
//! key. Schema 2 is refused by name.
//!
//! The verity layout (format 1, SHA-256, 4 KiB blocks, no superblock, the tree
//! appended after the data) is fixed by convention and lives in `atlas-image`.
//!
//! The main entry points:
//!
//! - [`bundle_from_image`]: open a first-flash image (through `atlas-image`),
//!   take a slot by id, write its bundle, in one call.
//! - [`write_bundle`]: make a bundle from any [`SlotSource`].
//! - [`inspect_bundle`]: read a bundle back with the board's checks; then
//!   [`check_board`] before anything is pushed.
//! - [`verify_slot`]: check a signature over `slot.json` on its own.
//!
//! The source is a seam, [`SlotSource`]. Its real implementation is
//! [`ImageSource`], over `atlas-image`'s slot. Tests use a double.
//!
//! What is checked, and what is not:
//!
//! - the four verity values against the layout, each with its own error
//!   (`atlas_image::VerityError`), and the file's length as the data plus its
//!   whole tree;
//! - the data, salted, against `verity.root_hash`, recomputed here (a wrong salt
//!   or a changed byte is refused);
//! - the tree's bytes are NOT recomputed. Its length is checked; its digest is
//!   the file's SHA-256 in the manifest.
//!
//! Trust. Signing protects INSTALL, not boot: `slot.sig` says a bundle came from
//! a trusted key, and a board refuses it only when `require_signed` is on. What
//! is trusted at boot is the root hash on the kernel command line, and only that.
//! Nothing in this crate calls a signed slot "verified boot", and nothing here
//! checks the signature or the tree at boot.
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
pub use verity::{BLOCK, VerityHasher, root_hash};
pub use write::{BOOT_DIR, ROOTFS, SLOT_JSON, SLOT_SIG, write_bundle};
