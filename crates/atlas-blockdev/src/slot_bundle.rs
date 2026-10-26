//! A slot bundle (`<id>.slot.tar`) made from a slots first-flash image
//! (`<id>.img.xz`, Gaia's devices/raze/gaia/slots.toml), so a board that
//! already has slots installs a new OS in place from the image the team was
//! given, rather than refusing it as "a whole-disk image".
//!
//! The image is an MBR disk: p1 FAT32 with the slot's boot files under
//! `slots/<id>/` (and its `slot.json`), p2 ext4 with the slot's root at
//! `slots/<id>.erofs`. The bundle is `slot.json`, `rootfs.erofs` and
//! `boot/<file>` for every file `slot.json`'s `boot.files` lists, each one
//! checked against its digest here (the board checks them all again). The
//! one file that differs by design is `cmdline.txt`: the FAT's carries the
//! slot arguments the writer adds, the bundle's is the base without them.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use sha2::{Digest, Sha256};

use crate::BlockError;
use crate::image::{decode_into, detect_format};

/// What the board's writer appends to the base cmdline for a slot.
const SLOT_ARGS: &str = " atlas.slot=";

/// The bundle made from an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotFromImage {
    /// The slot id (`slot.json`'s `id`).
    pub id: String,
    /// `<out_dir>/<id>.slot.tar`.
    pub path: PathBuf,
}

/// Whether `path` names a slot bundle rather than an image.
pub fn is_slot_bundle(path: &Path) -> bool {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".slot.tar")
}

/// Makes `<out_dir>/<id>.slot.tar` from the slots image at `image` (raw,
/// xz, zstd or gzip). A bundle already there for the same id is kept, and
/// any other one in `out_dir` (Atlas's own cache) goes, so only the latest
/// takes room. `out_dir` also holds the unpacked disk while this runs
/// (sparse: about the size of the slot).
pub fn slot_bundle_from_image(image: &Path, out_dir: &Path) -> Result<SlotFromImage, BlockError> {
    std::fs::create_dir_all(out_dir).map_err(|error| BlockError::io(out_dir, error))?;
    let disk_path = out_dir.join(format!(".unpack-{}.img", std::process::id()));
    let result =
        unpack_disk(image, &disk_path).and_then(|()| bundle_from_disk(&disk_path, out_dir));
    let _ = std::fs::remove_file(&disk_path);
    if let Ok(made) = &result
        && let Ok(entries) = std::fs::read_dir(out_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path != made.path && is_slot_bundle(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    result
}

fn not_slots(why: &str) -> BlockError {
    BlockError::Image(format!(
        "this isn't a slots image ({why}); a slot installs from a slots build's .img.xz or .slot.tar"
    ))
}

/// Decodes the image into a sparse file: runs of zeros are skipped, so the
/// empty part of the 1 GiB boot partition takes no room.
fn unpack_disk(image: &Path, disk_path: &Path) -> Result<(), BlockError> {
    let format = detect_format(image)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(disk_path)
        .map_err(|error| BlockError::io(disk_path, error))?;
    let mut sink = Sparse { file, len: 0 };
    decode_into(image, format, &mut sink, Arc::new(AtomicU64::new(0)))
        .map_err(|error| BlockError::Image(format!("{}: {error}", image.display())))?;
    sink.file
        .set_len(sink.len)
        .map_err(|error| BlockError::io(disk_path, error))
}

struct Sparse {
    file: File,
    len: u64,
}

impl Write for Sparse {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.iter().all(|&b| b == 0) {
            self.file.seek(SeekFrom::Current(buf.len() as i64))?;
        } else {
            self.file.write_all(buf)?;
        }
        self.len += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// One MBR partition's byte range.
#[derive(Clone, Copy, Debug)]
struct Partition {
    start: u64,
    len: u64,
}

fn partitions(disk: &mut File) -> io::Result<[Partition; 2]> {
    let mut mbr = [0u8; 512];
    disk.seek(SeekFrom::Start(0))?;
    disk.read_exact(&mut mbr)?;
    if mbr[510..512] != [0x55, 0xAA] {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no MBR"));
    }
    let entry = |n: usize| {
        let e = &mbr[446 + 16 * n..446 + 16 * (n + 1)];
        let lba = u32::from_le_bytes([e[8], e[9], e[10], e[11]]);
        let sectors = u32::from_le_bytes([e[12], e[13], e[14], e[15]]);
        Partition {
            start: u64::from(lba) * 512,
            len: u64::from(sectors) * 512,
        }
    };
    Ok([entry(0), entry(1)])
}

/// A partition of the disk file as its own seekable volume.
struct Slice {
    file: File,
    part: Partition,
    pos: u64,
}

impl Read for Slice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.part.len.saturating_sub(self.pos);
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        if want == 0 {
            return Ok(0);
        }
        self.file
            .seek(SeekFrom::Start(self.part.start + self.pos))?;
        let read = self.file.read(&mut buf[..want])?;
        if read == 0 {
            // Past the end of a truncated disk: the rest reads as zeros.
            buf[..want].fill(0);
            self.pos += want as u64;
            return Ok(want);
        }
        self.pos += read as u64;
        Ok(read)
    }
}

impl Write for Slice {
    // The FAT library wants a writable volume; the unpacked disk is a
    // throwaway copy, so whatever it writes is harmless.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file
            .seek(SeekFrom::Start(self.part.start + self.pos))?;
        let written = self.file.write(buf)?;
        self.pos += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Slice {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(to) => Some(to),
            SeekFrom::End(by) => self.part.len.checked_add_signed(by),
            SeekFrom::Current(by) => self.pos.checked_add_signed(by),
        };
        self.pos =
            to.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        Ok(self.pos)
    }
}

impl ext4_view::Ext4Read for Slice {
    fn read(
        &mut self,
        start_byte: u64,
        dst: &mut [u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.pos = start_byte;
        self.read_exact(dst).map_err(Box::new)?;
        Ok(())
    }
}

fn slice(disk: &Path, part: Partition) -> Result<Slice, BlockError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk)
        .map_err(|error| BlockError::io(disk, error))?;
    Ok(Slice { file, part, pos: 0 })
}

fn read_all(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn bundle_from_disk(disk_path: &Path, out_dir: &Path) -> Result<SlotFromImage, BlockError> {
    let image_error = |error: io::Error| BlockError::Image(error.to_string());
    let mut disk = File::open(disk_path).map_err(|error| BlockError::io(disk_path, error))?;
    let [boot_part, data_part] =
        partitions(&mut disk).map_err(|_| not_slots("no partition table"))?;
    if boot_part.len == 0 || data_part.len == 0 {
        return Err(not_slots("it needs a boot and a data partition"));
    }

    let fat = fatfs::FileSystem::new(slice(disk_path, boot_part)?, fatfs::FsOptions::new())
        .map_err(|_| not_slots("its first partition isn't FAT"))?;
    let slots_dir = fat
        .root_dir()
        .open_dir("slots")
        .map_err(|_| not_slots("its boot partition has no slots/"))?;
    let ids: Vec<String> = slots_dir
        .iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.is_dir())
        .map(|entry| entry.file_name())
        .filter(|name| name != "." && name != "..")
        .collect();
    let [id] = ids.as_slice() else {
        return Err(not_slots(&format!(
            "its boot partition has {} slots, not one",
            ids.len()
        )));
    };
    let slot_dir = slots_dir.open_dir(id).map_err(image_error)?;
    let manifest_bytes = read_all(
        slot_dir
            .open_file("slot.json")
            .map_err(|_| not_slots("the slot has no slot.json"))?,
    )
    .map_err(image_error)?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| BlockError::Image(format!("the image's slot.json: {error}")))?;
    if manifest["id"].as_str() != Some(id.as_str()) {
        return Err(BlockError::Image(format!(
            "the image's slot.json names another slot than slots/{id}"
        )));
    }
    let boot_files = manifest["boot"]["files"]
        .as_object()
        .ok_or_else(|| BlockError::Image("the image's slot.json lists no boot files".into()))?;
    let image_size = manifest["image"]["size"]
        .as_u64()
        .ok_or_else(|| BlockError::Image("the image's slot.json has no image size".into()))?;

    let path = out_dir.join(format!("{id}.slot.tar"));
    if path.exists() {
        return Ok(SlotFromImage {
            id: id.clone(),
            path,
        });
    }

    let ext4 = ext4_view::Ext4::load(Box::new(slice(disk_path, data_part)?))
        .map_err(|_| not_slots("its second partition isn't ext4"))?;
    let root_path = format!("/slots/{id}.erofs");
    let root = ext4
        .open(root_path.as_str())
        .map_err(|_| not_slots(&format!("its data partition has no {root_path}")))?;
    if root.metadata().len() != image_size {
        return Err(BlockError::Image(format!(
            "the image's {root_path} isn't the size its slot.json says"
        )));
    }

    let partial = out_dir.join(format!("{id}.slot.tar.partial"));
    let written = (|| -> Result<(), BlockError> {
        let out = File::create(&partial).map_err(|error| BlockError::io(&partial, error))?;
        let mut tar = tar::Builder::new(io::BufWriter::new(out));
        let mut add = |name: &str, len: u64, data: &mut dyn Read| -> Result<(), BlockError> {
            let mut header = tar::Header::new_ustar();
            header.set_size(len);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            tar.append_data(&mut header, name, data)
                .map_err(|error| BlockError::io(&partial, error))
        };
        add(
            "slot.json",
            manifest_bytes.len() as u64,
            &mut manifest_bytes.as_slice(),
        )?;
        for (name, want) in boot_files {
            let mut bytes = read_all(slot_dir.open_file(name).map_err(|_| {
                BlockError::Image(format!("the image's slot has no boot file {name}"))
            })?)
            .map_err(image_error)?;
            if name == "cmdline.txt"
                && let Some(base) = std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|line| line.find(SLOT_ARGS).map(|at| format!("{}\n", &line[..at])))
            {
                bytes = base.into_bytes();
            }
            if want.as_str() != Some(hex(&Sha256::digest(&bytes)).as_str()) {
                return Err(BlockError::Image(format!(
                    "the image's boot file {name} doesn't match its slot.json"
                )));
            }
            add(
                &format!("boot/{name}"),
                bytes.len() as u64,
                &mut bytes.as_slice(),
            )?;
        }
        let mut root = root;
        add("rootfs.erofs", image_size, &mut root)?;
        tar.into_inner()
            .and_then(|mut out| out.flush())
            .map_err(|error| BlockError::io(&partial, error))
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    std::fs::rename(&partial, &path).map_err(|error| BlockError::io(&path, error))?;
    Ok(SlotFromImage {
        id: id.clone(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_slot_tars_are_bundles() {
        assert!(is_slot_bundle(Path::new(
            "/x/photonvision-2026.3.4-styx-0d286005.slot.tar"
        )));
        assert!(!is_slot_bundle(Path::new(
            "/x/photonvision-2026.3.4-styx-0d286005.img.xz"
        )));
    }

    #[test]
    fn a_disk_without_slots_is_refused_and_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!("atlas-slot-bundle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("plain.img");
        std::fs::write(&image, vec![7u8; 4096]).unwrap();
        let out = dir.join("out");
        let error = slot_bundle_from_image(&image, &out)
            .unwrap_err()
            .to_string();
        assert!(error.contains("isn't a slots image"), "{error}");
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
