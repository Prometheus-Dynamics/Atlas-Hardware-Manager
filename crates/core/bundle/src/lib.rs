//! Slot bundles: one slot of a first-flash image, made into the tar a board's
//! slot writer takes, and read back the way that writer reads it.
//!
//! A bundle is a plain tar. Its members, in this order (Lemnos I5g2, I5i4):
//!
//! 1. `slot.json` (schema 3), always first;
//! 2. `slot.sig`, second when present: the 108-byte signature record, in the
//!    first-flash record's form (`LMNSSIG1`, the key id, the 64-byte signature);
//! 3. `rootfs.erofs` (the EROFS image with its dm-verity tree appended), and the
//!    `boot/...` files, in any order, each at most once.
//!
//! An unsigned bundle has no `slot.sig` and is legal. Nothing else is a member.
//!
//! The signed message is not the bare `slot.json`: [`slot_message`] builds
//! `lemnos-slot-bundle\0` | u32 LE version | u32 LE length | the exact `slot.json`
//! bytes. A signature over the bare bytes does not verify.
//!
//! `slot.json` carries `product`, `version`, optional `variant`, `board` (the
//! board file's `[board] id`, the board the slot was built for; a bundle
//! without it, or for another board, is refused), `id`, `kernel`,
//! `display_name`, `image.*`, `verity.*` (the root hash, the salt, the data
//! block count and the tree's offset) and `boot.files`. There is no `family`
//! key. Schema 2 is refused by name.
//!
//! `boot/` carries exactly the five files of [`BOOT_FILES`] (`kernel8.img`, `initramfs.cpio`,
//! `board.dtb`, `cmdline.txt`, `overlays/lemnos-board.dtbo`), each listed in `boot.files`. Lemnos
//! I5i8 (4e871efd). Each file's size is the board's bay (Lemnos checks it at install); the writer
//! does not know the bays.
//!
//! `boot/cmdline.txt` is the complete command line and carries one word
//! `<boot_args>.src.<source>=<kernel path>` for each boot source the board states
//! ([`BootRule`]); the installer appends nothing.
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
//! - [`verify_slot`]: check a `slot.sig` record against `slot.json` on its own.
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
//! Trust. A held key that does not verify refuses the bundle. A key the reader
//! does not hold is reported as [`Signature::UnknownKey`] and not refused: the
//! board's security switch decides what that means at install, not this crate.
//! What is trusted at boot is the root hash on the kernel command line, and only
//! that. Nothing in this crate calls a signed slot "verified boot".
//!
//! Untested on board.

mod boot;
mod error;
mod from_image;
mod inspect;
mod manifest;
mod sign;
mod source;
mod verity;
mod write;

pub use boot::BootRule;
pub use error::BundleError;
pub use from_image::{ImageSource, bundle_from_image, check_stream_digest};
pub use inspect::{Inspected, check_board, inspect_bundle};
pub use manifest::{
    BOOT_FILES, BootEntry, CMDLINE, ImageEntry, SCHEMA, SlotManifest, SlotMeta, VerityEntry,
    check_boot_set, validate_boot_path, validate_id, validate_name,
};
pub use sign::{
    RECORD_LEN, SIG_LEN, SLOT_MESSAGE_VERSION, SLOT_TAG, Signature, key_id_of, sign_slot,
    slot_message, verify_slot,
};
pub use source::{BootFile, SlotSource};
pub use verity::{BLOCK, VerityHasher, root_hash};
pub use write::{BOOT_DIR, ROOTFS, SLOT_JSON, SLOT_SIG, write_bundle};
