//! Reads one slot out of a first-flash image.
//!
//! The first-flash `.img.xz` is the only build output of a board; a slot bundle
//! is made from it. This crate is the reader behind a bundle's source: it takes
//! an image (xz-compressed, or a plain `.img`) and gives back, for one slot id,
//! the verity table text, the slot's file (the EROFS image with its dm-verity
//! tree appended), and the slot's boot files, each byte for byte as the image
//! holds them.
//!
//! # How it reads
//!
//! [`open`] makes one pass over the image, in disk order, because a compressed
//! stream cannot seek:
//!
//! 1. The GPT at the start (header and entry array, both CRC-checked) gives the
//!    partitions and the form (see [`Form`]).
//! 2. The boot partition is spooled to a scratch file, its boot files are copied
//!    out (for the slots asked for), and the spool is deleted.
//! 3. The data partition is spooled to a scratch file and kept. The slot file and
//!    the verity table are read from it with random access, so the data
//!    partition's whole length is the disk the reader needs, in the scratch
//!    directory (never more than [`Options::max_spool_bytes`]).
//! 4. The stream is decoded to its end, so the xz block checks cover the whole
//!    image, then the scratch directory is the only thing kept.
//!
//! The scratch directory is removed when the [`Image`] is dropped, and also when
//! [`open`] fails part way (the locals are dropped on every exit).
//!
//! Every later read (a bundle writer reads the slot's file twice, once to hash
//! and once to write) reads the spool or the extracted files again. Nothing is
//! decoded twice.
//!
//! # Forms
//!
//! - [`Form::BringUp`]: partitions `rootfs` (ext4) and `data` (ext4). The boot
//!   files are the rootfs's `/boot`, shared by every slot. The slot file is on
//!   `data`.
//! - [`Form::Real`]: partitions `boot` (FAT) and `data` (ext4). The boot files of
//!   slot `<id>` are the boot partition's `slots/<id>/`. The slot file is on
//!   `data`.
//!
//! Gaia does not yet fix every path these forms use; they are [`Names`] with the
//! defaults listed there, and each is open until Gaia writes the image.
//!
//! # Limits
//!
//! - The image is untrusted: every structure is bounds-checked, no allocation
//!   follows a size from the image without a cap, and names from the image
//!   (slot ids, file names) are checked before a path is made from them.
//! - The partition table is held in memory up to 1 MiB.
//! - Boot files: [`Options::max_file_bytes`] each and
//!   [`Options::max_boot_bytes`] in total.
//! - The decoder is `lzma-rs`, pure Rust. Its xz path has no memory limit, so
//!   its dictionary buffer grows with the stream's back-references, up to the
//!   dictionary size each block declares. It does not support the SHA-256 xz
//!   check (Gaia uses CRC-64); such an image is refused as undecodable.

mod budget;
mod error;
mod ext;
mod fat;
mod gpt;
mod names;
mod route;
mod scratch;

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use ext4_view::Ext4;

pub use error::ImageError;

use crate::route::{BootSet, BootSets, Router};
use crate::scratch::Scratch;

const XZ_MAGIC: &[u8] = &[0xFD, b'7', b'z', b'X', b'Z', 0x00];
/// The largest verity table read: Gaia's table is a few hundred bytes.
const MAX_TABLE_BYTES: u64 = 64 * 1024;

/// The two layouts this reader knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Gaia's bring-up form: partitions `rootfs` and `data`, both ext4. The boot
    /// files are the rootfs's `/boot`.
    BringUp,
    /// The real form: partitions `boot` (FAT) and `data` (ext4). The boot files
    /// of a slot are the boot partition's `slots/<id>/`.
    Real,
}

/// Where the reader finds things in the image. Each pattern may hold `{id}`,
/// which becomes the slot id. The defaults are the ones found in use or written
/// down, and each is open until Gaia's image fixes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Names {
    /// The bring-up root partition's name. Default `rootfs`.
    pub rootfs: String,
    /// The real form's boot partition's name, and the name that selects it.
    /// Default `boot`.
    pub boot: String,
    /// The data partition's name in both forms. Default `data`.
    pub data: String,
    /// The slot file on the data partition, relative to its root. Default
    /// `slots/{id}`, as the coordinator's facts give it. Open: dev boots
    /// `slots/{id}.erofs` and the zero 3E demo product names `slots/root.img`.
    pub slot_file: String,
    /// The verity table text on the data partition. Default `slots/{id}.verity`.
    /// Open: Gaia's `slot` recipe writes the table as its own output, not into
    /// the image, so no image carries one yet.
    pub verity_file: String,
    /// The real form's boot directory for a slot, on the boot partition. Default
    /// `slots/{id}`. Dev's first-flash disks use `slots/<id>/`.
    pub boot_dir: String,
    /// The bring-up form's boot directory, in the rootfs. Default `boot`.
    pub bringup_boot_dir: String,
}

impl Default for Names {
    fn default() -> Self {
        Self {
            rootfs: "rootfs".into(),
            boot: "boot".into(),
            data: "data".into(),
            slot_file: "slots/{id}".into(),
            verity_file: "slots/{id}.verity".into(),
            boot_dir: "slots/{id}".into(),
            bringup_boot_dir: "boot".into(),
        }
    }
}

/// What the reader is allowed to spend, and where it looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub names: Names,
    /// The largest partition the reader spools. Its length is the disk it
    /// needs in the scratch directory. Default 8 GiB.
    pub max_spool_bytes: u64,
    /// The most bytes of boot files copied out in total. Default 512 MiB.
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

/// A boot file of a slot: its path under `boot/` and its size in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootEntry {
    pub path: String,
    pub size: u64,
}

/// A first-flash image read once and ready to give out its slots. Holds the
/// scratch directory, so dropping it removes the spool and the extracted files.
pub struct Image {
    form: Form,
    names: Names,
    partitions: Vec<PartitionInfo>,
    requested: Vec<String>,
    boot: BootSets,
    data: Ext4,
    scratch: Scratch,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("form", &self.form)
            .field("partitions", &self.partitions)
            .field("requested", &self.requested)
            .finish_non_exhaustive()
    }
}

/// Reads the image at `image` (`.img.xz` or `.img`) for the slots `slots`, with
/// its scratch directory made under `scratch_parent`.
///
/// Every slot in `slots` must be in the image (its slot file), and a slot not
/// listed here cannot be read from the result. The image is read once, here; the
/// returned value gives out the parts without reading the image again.
pub fn open(
    image: &Path,
    scratch_parent: &Path,
    slots: &[&str],
    options: &Options,
) -> Result<Image, ImageError> {
    for id in slots {
        names::check_id(id)?;
    }
    let scratch = Scratch::create(scratch_parent)?;
    let file = File::open(image).map_err(|error| ImageError::io("the image", &error))?;
    let mut input = BufReader::with_capacity(1 << 20, file);
    let xz = input
        .fill_buf()
        .map_err(|error| ImageError::io("the image", &error))?
        .starts_with(XZ_MAGIC);
    let requested: Vec<String> = slots.iter().map(|id| (*id).to_string()).collect();
    let mut router = Router::new(scratch.path(), requested.clone(), options);
    let stream = if xz {
        lzma_rs::xz_decompress(&mut input, &mut router).map_err(|error| format!("{error:?}"))
    } else {
        io::copy(&mut input, &mut router)
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    let output = router.complete(stream)?;
    let data = ext::load(&output.data)?;
    for id in &requested {
        let path = slot_path(&options.names.slot_file, id)?;
        if !ext::is_file(&data, &path)? {
            return Err(ImageError::Missing(format!(
                "the data partition has no slot {id:?} at {path}"
            )));
        }
    }
    Ok(Image {
        form: output.form,
        names: options.names.clone(),
        partitions: output.partitions,
        requested,
        boot: output.boot,
        data,
        scratch,
    })
}

impl Image {
    /// The layout the image was read as.
    pub fn form(&self) -> Form {
        self.form
    }

    /// Every partition in the table, in disk order.
    pub fn partitions(&self) -> &[PartitionInfo] {
        &self.partitions
    }

    /// The scratch directory this image's spool and boot files live in.
    pub fn scratch_dir(&self) -> &Path {
        self.scratch.path()
    }

    /// One slot of the image. Fails if the id was not asked for at [`open`].
    pub fn slot(&self, id: &str) -> Result<Slot<'_>, ImageError> {
        names::check_id(id)?;
        if !self.requested.iter().any(|asked| asked == id) {
            return Err(ImageError::Missing(format!(
                "slot {id:?} was not asked for when the image was read"
            )));
        }
        Ok(Slot {
            image: self,
            id: id.to_string(),
        })
    }

    fn boot_set(&self, id: &str) -> Result<&BootSet, ImageError> {
        let key = match self.form {
            Form::BringUp => "",
            Form::Real => id,
        };
        self.boot
            .get(key)
            .ok_or_else(|| ImageError::Missing(format!("no boot files for slot {id:?}")))
    }
}

/// One slot of an image: the parts a slot bundle is made of.
pub struct Slot<'a> {
    image: &'a Image,
    id: String,
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

    /// The verity table: Gaia's `verity.table` text, as the image holds it.
    pub fn verity_table(&self) -> Result<String, ImageError> {
        let path = slot_path(&self.image.names.verity_file, &self.id)?;
        ext::read_text(&self.image.data, &path, MAX_TABLE_BYTES)
    }

    /// The length of the slot file: the EROFS image plus its verity tree.
    pub fn rootfs_len(&self) -> Result<u64, ImageError> {
        let path = slot_path(&self.image.names.slot_file, &self.id)?;
        ext::len(&self.image.data, &path)
    }

    /// The slot file, from its first byte. Each call reads it again from the
    /// spool.
    pub fn rootfs(&self) -> Result<Box<dyn Read>, ImageError> {
        let path = slot_path(&self.image.names.slot_file, &self.id)?;
        ext::open(&self.image.data, &path)
    }

    /// The boot files of the slot, by path under `boot/`, sorted.
    pub fn boot_files(&self) -> Result<Vec<BootEntry>, ImageError> {
        Ok(self
            .image
            .boot_set(&self.id)?
            .iter()
            .map(|(path, file)| BootEntry {
                path: path.clone(),
                size: file.size,
            })
            .collect())
    }

    /// One boot file, from its first byte. `path` is relative under `boot/`.
    pub fn boot_file(&self, path: &str) -> Result<Box<dyn Read>, ImageError> {
        names::split_rel(path)?;
        let file = self.image.boot_set(&self.id)?.get(path).ok_or_else(|| {
            ImageError::Missing(format!("no boot file {path:?} for slot {:?}", self.id))
        })?;
        let handle =
            File::open(&file.path).map_err(|error| ImageError::io("a boot file", &error))?;
        Ok(Box::new(handle))
    }
}

/// The path of a slot's file or table on the data partition: the pattern with
/// the id filled in, checked, made absolute.
fn slot_path(pattern: &str, id: &str) -> Result<String, ImageError> {
    let rel = names::expand(pattern, id);
    names::split_rel(&rel)?;
    Ok(format!("/{rel}"))
}
