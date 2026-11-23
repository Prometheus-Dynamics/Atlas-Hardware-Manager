//! A small first-flash image for tests: the one OS image a person installs from.
//!
//! The image is a GPT disk whose one partition, `data`, is an ext4 filesystem in Lemnos's layout
//! as Gaia writes it, holding one slot:
//!
//! - `slots/<id>.json`: the slot's manifest, the bytes a bundle carries as `slot.json`;
//! - `slots/<id>.sig`: the kept `slot.sig` record, when the slot is signed;
//! - `slots/<id>.erofs`: the slot file (one 4096-byte data block: a one-block slot has no tree);
//! - `slots/<id>.boot`: the boot list, `sha256sum -c` form, the manifest included as `slot.json`;
//! - `slots/bootfiles/<sha256>`: each boot file, and the manifest, by content.
//!
//! The manifest and the signature are made by `atlas-bundle`'s own writer (Lemnos's formats), so
//! what `atlas-image` reads back and `atlas_bundle::bundle_from_image` rebuilds is what a board
//! takes. The filesystem is made by `mke2fs -d`, with no root and no mount, as `atlas-image`'s own
//! test images are: a test that calls [`build`] needs e2fsprogs.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read};
use std::path::Path;
use std::process::Command;

use atlas_bundle::{BootFile, BundleError, SlotMeta, SlotSource};
use atlas_image::Verity;

use crate::bundle::sha256_hex;

/// The data block of a slot: 4096 bytes, so the slot has no verity tree.
const BLOCK: usize = 4096;
/// The data partition's size.
const DATA_BYTES: usize = 512 * 1024;
const FIRST_LBA: u64 = 2048;
/// A fixed salt: the images are test images.
const SALT: [u8; 32] = [0x5a; 32];

/// One slot to put in an image.
#[derive(Clone, Debug)]
pub struct Slot<'a> {
    /// The slot's id.
    pub id: &'a str,
    /// Its product, lowercase `[a-z0-9-]`.
    pub product: &'a str,
    /// Its version.
    pub version: &'a str,
    /// The board it is for.
    pub board: &'a str,
    /// The name a person reads, when it has one.
    pub display_name: Option<&'a str>,
    /// The Ed25519 secret that signs its manifest; `None` keeps the slot unsigned.
    pub signer: Option<[u8; 32]>,
}

struct Source {
    data: Vec<u8>,
    boot: BTreeMap<String, Vec<u8>>,
}

impl SlotSource for Source {
    fn verity(&self) -> Result<Verity, BundleError> {
        Ok(Verity {
            root_hash: atlas_bundle::root_hash(&SALT, &self.data)?,
            salt: SALT,
            data_blocks: 1,
            hash_offset: BLOCK as u64,
        })
    }

    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError> {
        Ok(Box::new(Cursor::new(&self.data)))
    }

    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError> {
        Ok(self
            .boot
            .iter()
            .map(|(path, bytes)| BootFile {
                path: path.clone(),
                size: bytes.len() as u64,
            })
            .collect())
    }

    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError> {
        self.boot
            .get(path)
            .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read + '_>)
            .ok_or_else(|| BundleError::Io(format!("no boot file {path}")))
    }
}

/// The files of `slot` on the data partition, by path. The slot's data is one block made from its
/// id and version, and its boot files are a command line and a stand-in kernel.
///
/// # Errors
///
/// When `atlas-bundle` refuses the slot (an id or a name it does not take).
pub fn files(slot: &Slot<'_>) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut data = format!("rootfs of {} at {}\n", slot.id, slot.version).into_bytes();
    data.resize(BLOCK, b'.');
    let source = Source {
        data,
        boot: BTreeMap::from([
            (
                "cmdline.txt".to_owned(),
                b"console=ttyS0 root=/dev/dm-0\n".to_vec(),
            ),
            ("kernel.img".to_owned(), b"kernel bytes".to_vec()),
        ]),
    };
    let meta = SlotMeta {
        product: slot.product.to_owned(),
        version: slot.version.to_owned(),
        board: slot.board.to_owned(),
        display_name: slot.display_name.map(str::to_owned),
        ..SlotMeta::default()
    };
    // The bundle writer always signs; an unsigned slot keeps no record, so the key is a throwaway.
    let key = slot.signer.unwrap_or([0x42; 32]);
    let mut tar = Vec::new();
    atlas_bundle::write_bundle(&source, slot.id, &meta, &key, &mut tar)
        .map_err(|error| format!("the slot cannot be made: {error}"))?;
    bundle_files(&tar, slot.id, slot.signer.is_some())
}

/// The files a data partition keeps for the slot `id` of the bundle `bundle` (a tar as a board
/// takes it), by path: what an image built with that slot holds. `keep_signature` keeps the
/// bundle's `slot.sig` as the image's kept record.
///
/// # Errors
///
/// When the bundle is not a tar or lacks its manifest or slot file.
pub fn bundle_files(
    bundle: &[u8],
    id: &str,
    keep_signature: bool,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut members = BTreeMap::new();
    let mut archive = tar::Archive::new(Cursor::new(bundle));
    for entry in archive.entries().map_err(|error| error.to_string())? {
        let mut entry = entry.map_err(|error| error.to_string())?;
        // The `boot/` directory entry is a directory, not a boot file (as Lemnos reads it).
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let name = entry
            .path()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .into_owned();
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        members.insert(name, bytes);
    }
    let take = |name: &str| {
        members
            .get(name)
            .cloned()
            .ok_or_else(|| format!("the slot has no {name}"))
    };
    let manifest = take(atlas_bundle::SLOT_JSON)?;
    let mut out = BTreeMap::new();
    out.insert(format!("slots/{id}.erofs"), take(atlas_bundle::ROOTFS)?);
    out.insert(format!("slots/{id}.json"), manifest.clone());
    if keep_signature {
        out.insert(format!("slots/{id}.sig"), take(atlas_bundle::SLOT_SIG)?);
    }
    // The boot list names every boot file and the manifest, each kept by its content.
    let mut listed: BTreeMap<String, Vec<u8>> = members
        .iter()
        .filter_map(|(name, bytes)| {
            name.strip_prefix(atlas_bundle::BOOT_DIR)
                .map(|path| (path.to_owned(), bytes.clone()))
        })
        .collect();
    listed.insert("slot.json".to_owned(), manifest);
    let mut list = String::new();
    for (path, bytes) in &listed {
        let digest = sha256_hex(bytes);
        list.push_str(&format!("{digest}  {path}\n"));
        out.insert(format!("slots/bootfiles/{digest}"), bytes.clone());
    }
    out.insert(format!("slots/{id}.boot"), list.into_bytes());
    Ok(out)
}

/// The first-flash image (`.img`, not compressed) holding `slot`, made in a scratch directory
/// under `scratch`.
///
/// # Errors
///
/// When the slot cannot be made, or `mke2fs` is missing or fails.
pub fn build(slot: &Slot<'_>, scratch: &Path) -> Result<Vec<u8>, String> {
    let files = files(slot)?;
    let data = data_partition(&files, scratch, DATA_BYTES).map_err(|error| {
        format!("the data partition cannot be made (mke2fs, from e2fsprogs, is needed): {error}")
    })?;
    Ok(disk(&data))
}

/// An ext4 filesystem of `bytes` bytes labelled `data` holding `files`, made by `mke2fs -d` in
/// `scratch`.
///
/// # Errors
///
/// When `mke2fs` is missing or fails.
pub fn data_partition(
    files: &BTreeMap<String, Vec<u8>>,
    scratch: &Path,
    bytes: usize,
) -> io::Result<Vec<u8>> {
    let stage = scratch.join("stage");
    for (path, bytes) in files {
        let full = stage.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(full, bytes)?;
    }
    let out = scratch.join("data.ext4");
    std::fs::write(&out, vec![0u8; bytes])?;
    let made = Command::new("mke2fs")
        .args([
            "-q",
            "-F",
            "-t",
            "ext4",
            "-b",
            "1024",
            "-N",
            "64",
            "-O",
            "^has_journal",
            "-L",
            "lemnos-data",
            "-d",
        ])
        .arg(&stage)
        .arg(&out)
        .output()?;
    if !made.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&made.stderr).trim().to_owned(),
        ));
    }
    std::fs::read(&out)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// A disk with a protective MBR, the primary GPT and one partition, `data`, holding `data`.
#[must_use]
pub fn disk(data: &[u8]) -> Vec<u8> {
    disk_of(&[("data", data)])
}

/// A disk with a protective MBR, the primary GPT and the partitions `parts` (name and bytes), one
/// after the other from sector 2048, each starting on a 1 MiB boundary.
#[must_use]
pub fn disk_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
    const ALIGN: u64 = 2048;
    let mut entries = vec![0u8; 128 * 128];
    let mut placed = Vec::new();
    let mut next = FIRST_LBA;
    for (index, (name, bytes)) in parts.iter().enumerate() {
        let sectors = (bytes.len() as u64).div_ceil(512).max(1);
        let entry = &mut entries[index * 128..(index + 1) * 128];
        entry[0..16].copy_from_slice(&[0x11; 16]);
        entry[16..32].copy_from_slice(&[index as u8 + 1; 16]);
        entry[32..40].copy_from_slice(&next.to_le_bytes());
        entry[40..48].copy_from_slice(&(next + sectors - 1).to_le_bytes());
        for (at, unit) in name.encode_utf16().enumerate() {
            entry[56 + at * 2..58 + at * 2].copy_from_slice(&unit.to_le_bytes());
        }
        placed.push(((next * 512) as usize, *bytes));
        next = (next + sectors).div_ceil(ALIGN) * ALIGN;
    }
    let end = placed
        .iter()
        .map(|(start, bytes)| start + bytes.len())
        .max()
        .unwrap_or((FIRST_LBA * 512) as usize);
    let mut disk = vec![0u8; end];
    disk[510] = 0x55;
    disk[511] = 0xAA;
    let mut header = vec![0u8; 92];
    header[0..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    header[12..16].copy_from_slice(&92u32.to_le_bytes());
    header[24..32].copy_from_slice(&1u64.to_le_bytes());
    header[40..48].copy_from_slice(&34u64.to_le_bytes());
    header[48..56].copy_from_slice(&(end as u64 / 512 - 34).to_le_bytes());
    header[56..72].copy_from_slice(&[0x22; 16]);
    header[72..80].copy_from_slice(&2u64.to_le_bytes());
    header[80..84].copy_from_slice(&128u32.to_le_bytes());
    header[84..88].copy_from_slice(&128u32.to_le_bytes());
    header[88..92].copy_from_slice(&crc32(&entries).to_le_bytes());
    let header_crc = crc32(&header);
    header[16..20].copy_from_slice(&header_crc.to_le_bytes());
    disk[512..512 + 92].copy_from_slice(&header);
    disk[1024..1024 + entries.len()].copy_from_slice(&entries);
    for (start, bytes) in placed {
        disk[start..start + bytes.len()].copy_from_slice(bytes);
    }
    disk
}
