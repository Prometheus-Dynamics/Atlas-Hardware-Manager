//! Reads one slot out of a first-flash image.
//!
//! The first-flash `.img.xz` is the only build output of a board; a slot bundle
//! is made from it. This crate reads a slot from an image (xz-compressed, or a
//! plain `.img`) in the layout Gaia writes for Lemnos: on the image's `data`
//! partition,
//!
//! - `slots/<id>.erofs`: the slot's file, the EROFS image with its dm-verity tree
//!   appended;
//! - `slots/<id>.json`: the slot's manifest (its `verity.root_hash`, `image.size`,
//!   `image.sha256`, and the boot files by sha256);
//! - `slots/<id>.boot`: the boot list, `<sha256>  <path>` per file, including
//!   `slot.json`;
//! - `slots/bootfiles/<sha256>`: the boot files, kept by content.
//!
//! The reader gives back the slot's file, its manifest and its boot files, each
//! byte for byte, and refuses anything that does not agree with its own names:
//! a boot file whose content is not the sha256 in its name, a manifest that
//! names another slot, a boot list that disagrees with the manifest, or an
//! erofs whose length is not `image.size`.
//!
//! # How it reads
//!
//! [`open`] makes one pass over the image, because a compressed stream cannot
//! seek. The partition table is read first, then the `data` partition is
//! spooled to a scratch file and kept. The slot files are read from that spool
//! with random access, and the rest of the stream is decoded and dropped. The
//! boot files are read from the spool as well, so nothing is copied out, and
//! every later read (a bundle writer reads a slot's file twice) reads the spool
//! again: nothing is decoded twice.
//!
//! The scratch directory is removed when the [`Image`] is dropped, and also when
//! [`open`] fails part way (the locals are dropped on every exit path).
//!
//! # Limits
//!
//! - The image is untrusted. Every structure is bounds-checked, no allocation
//!   follows a size from the image without a cap, and names from the image are
//!   checked before a path is made from them.
//! - The partition table is held in memory up to 1 MiB.
//! - Disk: the `data` partition's length, in the scratch directory, capped by
//!   [`Options::max_spool_bytes`] (default 8 GiB).
//! - Boot files: [`Options::max_file_bytes`] each and
//!   [`Options::max_boot_bytes`] in total, read once at open to be hashed.
//! - The decoder is `lzma-rs`, pure Rust. Its xz path has no memory limit: its
//!   dictionary buffer grows with the stream's back-references, up to the
//!   dictionary size each block declares. It does not support the SHA-256 xz
//!   check (Gaia uses CRC-64); such an image is refused as undecodable.
//!
//! # What is not checked here
//!
//! The erofs file's own bytes are not hashed at open (a slot can be gigabytes):
//! [`Slot::image_sha256`] is the manifest's claim, and the bundle writer or the
//! board's verity check is what proves it. The verity tree is not recomputed:
//! the manifest's root hash is exposed as written.

mod budget;
mod error;
mod ext;
mod gpt;
mod manifest;
mod names;
mod route;
mod scratch;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use ext4_view::Ext4;
use sha2::{Digest, Sha256};

pub use error::ImageError;

use crate::budget::Budget;
use crate::manifest::Manifest;
use crate::route::Router;
use crate::scratch::Scratch;

const XZ_MAGIC: &[u8] = &[0xFD, b'7', b'z', b'X', b'Z', 0x00];
/// The largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The largest boot list read.
const MAX_BOOT_LIST_BYTES: u64 = 1 << 20;
/// Read size when a boot file is hashed.
const CHUNK: usize = 64 * 1024;

/// Where the reader finds a slot's files on the data partition. Each pattern may
/// hold `{id}`, which becomes the slot id. The defaults are Lemnos's layout as
/// Gaia writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Names {
    /// The data partition's name. Default `data`.
    pub data: String,
    /// The slot file. Default `slots/{id}.erofs`.
    pub slot_file: String,
    /// The manifest. Default `slots/{id}.json`.
    pub manifest_file: String,
    /// The boot list. Default `slots/{id}.boot`.
    pub boot_list: String,
    /// The directory of boot files kept by content. Default `slots/bootfiles`.
    pub boot_store: String,
}

impl Default for Names {
    fn default() -> Self {
        Self {
            data: "data".into(),
            slot_file: "slots/{id}.erofs".into(),
            manifest_file: "slots/{id}.json".into(),
            boot_list: "slots/{id}.boot".into(),
            boot_store: "slots/bootfiles".into(),
        }
    }
}

/// What the reader is allowed to spend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub names: Names,
    /// The largest data partition the reader spools; its length is the scratch
    /// disk the read needs. Default 8 GiB.
    pub max_spool_bytes: u64,
    /// The most boot-file bytes hashed for one slot. Default 512 MiB.
    pub max_boot_bytes: u64,
    /// The most bytes of one boot file. Default 256 MiB.
    pub max_file_bytes: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            names: Names::default(),
            max_spool_bytes: 8 << 30,
            max_boot_bytes: 512 << 20,
            max_file_bytes: 256 << 20,
        }
    }
}

/// One partition of the image: its name and where its bytes are, in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionInfo {
    pub name: String,
    pub start: u64,
    pub len: u64,
}

/// A boot file of a slot: its path under `boot/`, its size and its sha256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// One boot file as the image holds it: its path in the image, size and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BootFile {
    store: String,
    size: u64,
    sha256: String,
}

/// What [`open`] checked for one slot.
#[derive(Clone, Debug)]
struct SlotData {
    manifest_json: String,
    manifest: Manifest,
    slot_file: String,
    boot: BTreeMap<String, BootFile>,
}

/// A first-flash image read once. Holds the scratch directory, so dropping it
/// removes the data spool.
pub struct Image {
    partitions: Vec<PartitionInfo>,
    slots: BTreeMap<String, SlotData>,
    data: Ext4,
    scratch: Scratch,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("partitions", &self.partitions)
            .field("slots", &self.slots.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Reads the image at `image` (`.img.xz` or `.img`) for the slots `slots`, with
/// its scratch directory made under `scratch_parent`.
///
/// Each slot in `slots` must be complete in the image: its manifest, its file of
/// the manifest's length, its boot list, and every boot file hashed against its
/// name. A slot not listed here cannot be read from the result.
pub fn open(
    image: &Path,
    scratch_parent: &Path,
    slots: &[&str],
    options: &Options,
) -> Result<Image, ImageError> {
    for id in slots {
        names::check_id(id)?;
    }
    let mut requested: Vec<String> = slots.iter().map(|id| (*id).to_string()).collect();
    requested.sort();
    requested.dedup();

    let scratch = Scratch::create(scratch_parent)?;
    let file = File::open(image).map_err(|error| ImageError::io("the image", &error))?;
    let mut input = BufReader::with_capacity(1 << 20, file);
    let xz = input
        .fill_buf()
        .map_err(|error| ImageError::io("the image", &error))?
        .starts_with(XZ_MAGIC);
    let mut router = Router::new(scratch.path(), options);
    let stream = if xz {
        lzma_rs::xz_decompress(&mut input, &mut router).map_err(|error| format!("{error:?}"))
    } else {
        io::copy(&mut input, &mut router)
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    let output = router.complete(stream)?;
    let data = ext::load(&output.data)?;

    let mut budget = Budget::new(options.max_boot_bytes, options.max_file_bytes);
    let mut parsed = BTreeMap::new();
    for id in &requested {
        parsed.insert(
            id.clone(),
            read_slot(&data, id, &options.names, &mut budget)?,
        );
    }
    Ok(Image {
        partitions: output.partitions,
        slots: parsed,
        data,
        scratch,
    })
}

/// Reads and checks one slot's manifest, file, boot list and boot files.
fn read_slot(
    fs: &Ext4,
    id: &str,
    names: &Names,
    budget: &mut Budget,
) -> Result<SlotData, ImageError> {
    let manifest_path = data_path(&names.manifest_file, id)?;
    let manifest_json = ext::read_text(fs, &manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest = manifest::parse_manifest(&manifest_json, id)?;

    let slot_file = data_path(&names.slot_file, id)?;
    let erofs_len = ext::len(fs, &slot_file)?;
    if erofs_len != manifest.image.size {
        return Err(ImageError::Malformed(format!(
            "slot {id:?}'s file is {erofs_len} bytes, its manifest says {}",
            manifest.image.size
        )));
    }

    let list_path = data_path(&names.boot_list, id)?;
    let list = manifest::parse_boot_list(&ext::read_text(fs, &list_path, MAX_BOOT_LIST_BYTES)?)?;
    for (path, sha) in &manifest.boot.files {
        let listed = list.iter().find(|line| &line.path == path);
        if listed.map(|line| &line.sha256) != Some(sha) {
            return Err(ImageError::Malformed(format!(
                "slot {id:?}: the manifest's boot file {path:?} is not in the boot list with the same digest"
            )));
        }
    }

    let mut boot = BTreeMap::new();
    for line in list {
        let store = data_path(&format!("{}/{}", names.boot_store, line.sha256), id)?;
        let size = ext::len(fs, &store)?;
        budget.take(size)?;
        let mut reader = ext::open(fs, &store)?;
        let digest =
            sha256_hex(&mut reader).map_err(|error| ImageError::io("a boot file", &error))?;
        if digest != line.sha256 {
            return Err(ImageError::Malformed(format!(
                "slot {id:?}: boot file {:?} is {digest}, not the {} its name says",
                line.path, line.sha256
            )));
        }
        boot.insert(
            line.path,
            BootFile {
                store,
                size,
                sha256: line.sha256,
            },
        );
    }
    Ok(SlotData {
        manifest_json,
        manifest,
        slot_file,
        boot,
    })
}

fn sha256_hex(reader: &mut dyn Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

impl Image {
    /// The partitions of the table, in disk order.
    pub fn partitions(&self) -> &[PartitionInfo] {
        &self.partitions
    }

    /// The scratch directory holding the data spool. It is removed on drop.
    pub fn scratch_dir(&self) -> &Path {
        self.scratch.path()
    }

    /// One slot of the image. Fails if the id was not asked for at [`open`].
    pub fn slot(&self, id: &str) -> Result<Slot<'_>, ImageError> {
        names::check_id(id)?;
        let data = self.slots.get(id).ok_or_else(|| {
            ImageError::Missing(format!(
                "slot {id:?} was not asked for when the image was read"
            ))
        })?;
        Ok(Slot {
            image: self,
            id: id.to_string(),
            data,
        })
    }
}

/// One slot of an image: the parts a slot bundle is made of.
pub struct Slot<'a> {
    image: &'a Image,
    id: String,
    data: &'a SlotData,
}

impl std::fmt::Debug for Slot<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Slot<'_> {
    /// The slot id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The manifest's `verity.root_hash`, lowercase hex, as written.
    pub fn root_hash(&self) -> &str {
        &self.data.manifest.verity.root_hash
    }

    /// The manifest's `image.sha256` (the slot file's digest), as written. Not
    /// recomputed at open; see the crate docs.
    pub fn image_sha256(&self) -> &str {
        &self.data.manifest.image.sha256
    }

    /// The manifest's `image.size`, which the slot file's length was checked
    /// against at open.
    pub fn image_size(&self) -> u64 {
        self.data.manifest.image.size
    }

    /// The manifest exactly as the image holds it (`slots/<id>.json`).
    pub fn manifest_json(&self) -> &str {
        &self.data.manifest_json
    }

    /// The slot file, from its first byte. Each call reads it again from the
    /// spool.
    pub fn rootfs(&self) -> Result<Box<dyn Read>, ImageError> {
        ext::open(&self.image.data, &self.data.slot_file)
    }

    /// The boot files of the slot, by path under `boot/` (`slot.json` included),
    /// sorted by path.
    pub fn boot_files(&self) -> Vec<BootEntry> {
        self.data
            .boot
            .iter()
            .map(|(path, file)| BootEntry {
                path: path.clone(),
                size: file.size,
                sha256: file.sha256.clone(),
            })
            .collect()
    }

    /// One boot file, from its first byte. `path` is relative under `boot/`.
    pub fn boot_file(&self, path: &str) -> Result<Box<dyn Read>, ImageError> {
        names::split_rel(path)?;
        let file = self.data.boot.get(path).ok_or_else(|| {
            ImageError::Missing(format!("no boot file {path:?} for slot {:?}", self.id))
        })?;
        ext::open(&self.image.data, &file.store)
    }
}

/// The path of a slot's file on the data partition: the pattern with the id
/// filled in, checked, made absolute.
fn data_path(pattern: &str, id: &str) -> Result<String, ImageError> {
    let rel = names::expand(pattern, id);
    names::split_rel(&rel)?;
    Ok(format!("/{rel}"))
}
