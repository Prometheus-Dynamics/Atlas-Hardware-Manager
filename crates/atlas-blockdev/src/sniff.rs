//! What an artifact is, from its content (docs/devices-as-data.md): the
//! kinds Gaia builds per device profile, told apart so Atlas can offer the
//! install methods that fit.
//!
//! - an image (raw, xz, zstd, gzip): only its first 32 MiB are decoded.
//!   A first FAT with `slots/`: a slots build's first-flash disk
//!   (slots.toml). An extended partition or three FATs or more (autoboot,
//!   boot A, boot B: raze/build.toml): the A/B layout. Anything else with a
//!   table (an older single-root image): a disk image for a fresh install.
//! - a tar with `slot.json`: a slot bundle; with `layers.json` or `layer.json`
//!   and no slot: a layer.
//! - an `ar` archive (`!<arch>`): a Debian package, or an opkg one when its
//!   name ends in `.ipk`.
//!
//! The method that installs it checks it fully; this only chooses which
//! methods to offer.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use atlas_driver::ArtifactKind;

use crate::image::{ImageFormat, decode_into, detect_format};

/// How much of an image is decoded: its partition table and, for a slots
/// disk, the boot FAT's root directory (after its two FATs, ~16 MiB in).
const HEAD: usize = 32 << 20;

/// The kind of the file at `path`; [`kind_from_name`] when it can't be read.
pub fn sniff_artifact(path: &Path) -> ArtifactKind {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(mut file) = File::open(path) else {
        return kind_from_name(&name);
    };
    let mut magic = [0u8; 8];
    let read = file.read(&mut magic).unwrap_or(0);
    if magic[..read].starts_with(b"!<arch>\n") {
        return if name.to_ascii_lowercase().ends_with(".ipk") {
            ArtifactKind::Ipk
        } else {
            ArtifactKind::Deb
        };
    }
    if is_tar(path) {
        return tar_kind(path);
    }
    let Ok(format) = detect_format(path) else {
        return kind_from_name(&name);
    };
    let Some(head) = image_head(path, format) else {
        return kind_from_name(&name);
    };
    match image_kind(&head) {
        Some(kind) => kind,
        // A compressed file that isn't a disk: a tar inside (an old-style
        // ipk is a tar.gz), or something else.
        None if format != ImageFormat::Raw && head.len() >= 512 && is_tar_header(&head[..512]) => {
            if name.to_ascii_lowercase().ends_with(".ipk") {
                ArtifactKind::Ipk
            } else {
                ArtifactKind::Unknown
            }
        }
        None => kind_from_name(&name),
    }
}

/// The kind a file's name suggests, for a release not downloaded yet.
pub fn kind_from_name(name: &str) -> ArtifactKind {
    let name = name.to_ascii_lowercase();
    let image = [".img", ".img.xz", ".img.zst", ".img.gz"]
        .iter()
        .any(|ext| name.ends_with(ext));
    if name.ends_with(".slot.tar") {
        ArtifactKind::SlotBundle
    } else if name.ends_with(".layer.tar") {
        ArtifactKind::Layer
    } else if name.ends_with(".ipk") {
        ArtifactKind::Ipk
    } else if name.ends_with(".deb") {
        ArtifactKind::Deb
    } else if image {
        // Can't tell slots from A/B without the content.
        ArtifactKind::DiskImage
    } else if [".bin", ".hex", ".uf2", ".dfu", ".elf"]
        .iter()
        .any(|ext| name.ends_with(ext))
    {
        ArtifactKind::Firmware
    } else {
        ArtifactKind::Unknown
    }
}

/// The first [`HEAD`] bytes of the image, decoded.
fn image_head(path: &Path, format: ImageFormat) -> Option<Vec<u8>> {
    let mut sink = Head(Vec::with_capacity(HEAD));
    match decode_into(path, format, &mut sink, Arc::new(AtomicU64::new(0))) {
        Ok(()) => {}
        // The sink stops the decoder once it has enough.
        Err(error) if error.kind() == io::ErrorKind::WriteZero => {}
        Err(_) if !sink.0.is_empty() => {}
        Err(_) => return None,
    }
    Some(sink.0)
}

struct Head(Vec<u8>);

impl Write for Head {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = HEAD - self.0.len();
        if room == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "enough"));
        }
        let take = room.min(buf.len());
        self.0.extend_from_slice(&buf[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The kind from an MBR's partitions; `None` without a table.
fn image_kind(head: &[u8]) -> Option<ArtifactKind> {
    if head.len() < 512 || head[510..512] != [0x55, 0xAA] {
        return None;
    }
    let types: Vec<u8> = (0..4)
        .map(|n| &head[446 + 16 * n..446 + 16 * (n + 1)])
        .filter(|e| u32::from_le_bytes([e[12], e[13], e[14], e[15]]) > 0)
        .map(|e| e[4])
        .collect();
    if types.is_empty() {
        return None;
    }
    let fat = |t: u8| matches!(t, 0x0b | 0x0c | 0x0e | 0x06);
    let extended = |t: u8| matches!(t, 0x05 | 0x0f | 0x85);
    if types.first().is_some_and(|t| fat(*t)) && first_fat_has_slots(head) {
        return Some(ArtifactKind::SlotsImage);
    }
    Some(
        if types.iter().any(|t| extended(*t)) || types.iter().filter(|t| fat(**t)).count() >= 3 {
            ArtifactKind::AbImage
        } else {
            ArtifactKind::DiskImage
        },
    )
}

/// Whether the first partition's FAT has a `slots` directory at its root,
/// read from the decoded head of the disk.
fn first_fat_has_slots(head: &[u8]) -> bool {
    let start = u32::from_le_bytes([head[454], head[455], head[456], head[457]]) as usize * 512;
    let Some(part) = head.get(start..) else {
        return false;
    };
    let volume = io::Cursor::new(part.to_vec());
    let Ok(fs) = fatfs::FileSystem::new(volume, fatfs::FsOptions::new()) else {
        return false;
    };
    fs.root_dir().open_dir("slots").is_ok()
}

fn is_tar_header(block: &[u8]) -> bool {
    block.len() >= 263 && &block[257..262] == b"ustar"
}

fn is_tar(path: &Path) -> bool {
    let mut block = [0u8; 512];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut block))
        .is_ok_and(|()| is_tar_header(&block))
}

/// A slot bundle or a layer, by the tar's member names (headers only).
fn tar_kind(path: &Path) -> ArtifactKind {
    let Ok(file) = File::open(path) else {
        return ArtifactKind::Unknown;
    };
    let mut archive = tar::Archive::new(file);
    let Ok(entries) = archive.entries() else {
        return ArtifactKind::Unknown;
    };
    let mut layer = false;
    for entry in entries.take(64).flatten() {
        let Ok(name) = entry.path() else { continue };
        let name = name.to_string_lossy();
        let name = name.trim_start_matches("./");
        if name == "slot.json" {
            return ArtifactKind::SlotBundle;
        }
        if name == "layer.json" || name == "layers.json" || name.ends_with("/layers.json") {
            layer = true;
        }
    }
    if layer {
        ArtifactKind::Layer
    } else {
        ArtifactKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// An MBR with these partition types (each 1 sector).
    fn mbr(types: &[u8]) -> Vec<u8> {
        let mut disk = vec![0u8; 4096];
        for (n, t) in types.iter().enumerate() {
            let e = 446 + 16 * n;
            disk[e + 4] = *t;
            disk[e + 8..e + 12].copy_from_slice(&(1 + n as u32).to_le_bytes());
            disk[e + 12..e + 16].copy_from_slice(&1u32.to_le_bytes());
        }
        disk[510] = 0x55;
        disk[511] = 0xAA;
        disk
    }

    #[test]
    fn images_are_told_apart_by_their_partitions() {
        // FAT + Linux without a slots FAT: an older single-root image.
        assert_eq!(
            sniff_artifact(&temp("a.img", &mbr(&[0x0c, 0x83]))),
            ArtifactKind::DiskImage
        );
        assert_eq!(
            sniff_artifact(&temp("b.img", &mbr(&[0x0c, 0x0c, 0x0c, 0x0f]))),
            ArtifactKind::AbImage
        );
        assert_eq!(
            sniff_artifact(&temp("c.img", &mbr(&[0x0c]))),
            ArtifactKind::DiskImage
        );
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut &mbr(&[0x0c, 0x0c, 0x0c, 0x05])[..], &mut xz).unwrap();
        assert_eq!(
            sniff_artifact(&temp("d.img.xz", &xz)),
            ArtifactKind::AbImage,
            "decoded, not by name"
        );
    }

    #[test]
    fn archives_by_their_members() {
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_ustar();
        header.set_size(2);
        header.set_mode(0o644);
        tar.append_data(&mut header, "slot.json", &b"{}"[..])
            .unwrap();
        let bytes = tar.into_inner().unwrap();
        assert_eq!(
            sniff_artifact(&temp("whatever.tar", &bytes)),
            ArtifactKind::SlotBundle
        );
        assert_eq!(
            sniff_artifact(&temp("p.ipk", b"!<arch>\ndebian-binary")),
            ArtifactKind::Ipk
        );
        assert_eq!(
            sniff_artifact(&temp("p.deb", b"!<arch>\ndebian-binary")),
            ArtifactKind::Deb
        );
    }

    #[test]
    fn names_stand_in_before_a_download() {
        assert_eq!(
            kind_from_name("photonvision-2026.3.4-styx-0d286005.slot.tar"),
            ArtifactKind::SlotBundle
        );
        assert_eq!(
            kind_from_name("helios-raze-2026.3.1.img.zst"),
            ArtifactKind::DiskImage
        );
        assert_eq!(kind_from_name("helios_1.0_aarch64.ipk"), ArtifactKind::Ipk);
        assert_eq!(kind_from_name("mcu-1.5.0.bin"), ArtifactKind::Firmware);
    }
}
