//! Test helpers: the deterministic contents the fixtures hold, a GPT and a FAT
//! writer, the xz wrapper, and a test directory that removes itself.

#![allow(dead_code)]

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use atlas_bundle::{BLOCK, root_hash};

pub const DATA_FIXTURE: &[u8] = include_bytes!("../fixtures/data.ext4");
pub const ROOTFS_FIXTURE: &[u8] = include_bytes!("../fixtures/rootfs.ext4");

/// The slot's salt: the fixture's table was made with it.
pub const SALT: [u8; 16] = [0x5A; 16];
/// Four data blocks, then one tree block.
pub const SLOT_DATA_LEN: usize = 4 * BLOCK;
pub const SLOT_TREE_LEN: usize = BLOCK;

/// The bytes the fixtures were made from (the same formula as the Python
/// generator, make-fixtures.py).
pub fn pattern(n: usize, seed: u64) -> Vec<u8> {
    (0..n as u64)
        .map(|i| ((i * 7 + seed * 13) % 251) as u8)
        .collect()
}

/// The slot file: the data blocks, then the verity tree.
pub fn slot_file() -> Vec<u8> {
    let mut bytes = pattern(SLOT_DATA_LEN, 1);
    bytes.extend_from_slice(&pattern(SLOT_TREE_LEN, 2));
    bytes
}

/// Gaia's verity table text for the slot, the root computed by the bundle
/// crate's own hasher (the fixture's table was computed by the Python script).
pub fn verity_text() -> String {
    let root = root_hash(&SALT, &slot_file()[..SLOT_DATA_LEN]).expect("root");
    let salt: String = SALT.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "hash_type 1\nalgorithm sha256\nroot_hash {root}\nsalt {salt}\ndata_blocks 4\n\
         data_block_size 4096\nhash_block_size 4096\nhash_offset {}\n",
        SLOT_DATA_LEN
    )
}

/// The bring-up form's boot files, by path under `boot/`: the rootfs's `/boot`.
pub fn bringup_boot() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Image", pattern(40_000, 3)),
        ("board.dtb", pattern(1234, 4)),
        (
            "extlinux/extlinux.conf",
            b"default l\nlabel l\n  kernel /boot/Image\n  fdt /boot/board.dtb\n  append root=PARTLABEL=rootfs ro\n"
                .to_vec(),
        ),
    ]
}

/// The real form's boot files of slot `alpha` on the FAT boot partition, by
/// path under `boot/`. Slot `beta` is there too, to check it is not read.
pub fn real_boot() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Image", pattern(40_000, 3)),
        ("board.dtb", pattern(1234, 4)),
        ("initramfs.cpio.gz", pattern(9000, 5)),
        (
            "cmdline.txt",
            b"console=tty1 root=PARTUUID=abc panic=5\n".to_vec(),
        ),
    ]
}

/// A test directory under the cargo target tmp dir, removed on drop.
pub struct TestDir(PathBuf);

static NEXT: AtomicU64 = AtomicU64::new(0);

impl TestDir {
    pub fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "atlas-image-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).expect("write");
        path
    }

    /// Entries in the directory: the scratch directories of a read sit here.
    pub fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .expect("read dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One partition for [`gpt_disk`]: its name, its first sector and its bytes.
pub struct Part<'a> {
    pub name: &'a str,
    pub first_lba: u64,
    pub bytes: &'a [u8],
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A disk: a protective MBR, the primary GPT (header at LBA 1, 128 entries at
/// LBA 2), and the partitions at their sectors. No backup table: the reader
/// does not need one.
pub fn gpt_disk(parts: &[Part<'_>]) -> Vec<u8> {
    let end = parts
        .iter()
        .map(|p| (p.first_lba * 512) as usize + p.bytes.len())
        .max()
        .unwrap_or(0)
        .max(64 * 512);
    let mut disk = vec![0u8; end];
    disk[510] = 0x55;
    disk[511] = 0xAA;

    let mut entries = vec![0u8; 128 * 128];
    for (index, part) in parts.iter().enumerate() {
        let entry = &mut entries[index * 128..(index + 1) * 128];
        entry[0..16].copy_from_slice(&[0x11; 16]);
        entry[16..32].copy_from_slice(&[index as u8 + 1; 16]);
        let last = part.first_lba + (part.bytes.len() as u64).div_ceil(512) - 1;
        entry[32..40].copy_from_slice(&part.first_lba.to_le_bytes());
        entry[40..48].copy_from_slice(&last.to_le_bytes());
        let (slots, _) = entry[56..128].as_chunks_mut::<2>();
        for (slot, unit) in slots.iter_mut().zip(part.name.encode_utf16()) {
            *slot = unit.to_le_bytes();
        }
    }
    let entries_crc = crc32(&entries);

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
    header[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    let header_crc = crc32(&header);
    header[16..20].copy_from_slice(&header_crc.to_le_bytes());

    disk[512..512 + 92].copy_from_slice(&header);
    disk[1024..1024 + entries.len()].copy_from_slice(&entries);
    for part in parts {
        let start = (part.first_lba * 512) as usize;
        disk[start..start + part.bytes.len()].copy_from_slice(part.bytes);
    }
    disk
}

/// A FAT volume of `size` bytes with the given files (paths with `/`).
pub fn fat_volume(size: u32, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    use fatfs::{FileSystem, FormatVolumeOptions, FsOptions};

    let mut cursor = Cursor::new(vec![0u8; size as usize]);
    fatfs::format_volume(
        &mut cursor,
        FormatVolumeOptions::new().total_sectors(size / 512),
    )
    .expect("format");
    cursor.set_position(0);
    let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("mount");
    for (path, bytes) in files {
        let mut dir = fs.root_dir();
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            dir = match dir.open_dir(part) {
                Ok(open) => open,
                Err(_) => {
                    dir.create_dir(part).expect("mkdir");
                    dir.open_dir(part).expect("open dir")
                }
            };
        }
        let mut file = dir.create_file(parts[parts.len() - 1]).expect("create");
        file.write_all(bytes).expect("write");
    }
    fs.unmount().expect("unmount");
    cursor.into_inner()
}

/// The xz file of `raw`, as the image's build writes it: liblzma's encoder (the
/// `xz` program) with its default CRC-64 block check, which is what Gaia's xz
/// writer produces. The pure-Rust encoder in lzma-rs writes no check at all, so
/// it cannot show that a damaged stream is refused.
pub fn xz(raw: &[u8]) -> Vec<u8> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("xz")
        .args(["--check=crc64", "--compress", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the xz program is needed to make the test images");
    let mut stdin = child.stdin.take().expect("stdin");
    let input = raw.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().expect("xz");
    writer.join().expect("writer").expect("write to xz");
    assert!(output.status.success(), "xz failed");
    output.stdout
}

/// The real form's disk: the FAT boot partition at LBA 2048 (2 MiB), holding
/// `slots/alpha/` and a decoy `slots/beta/`, then the data partition after it.
pub fn real_disk() -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = real_boot()
        .into_iter()
        .map(|(name, bytes)| (format!("slots/alpha/{name}"), bytes))
        .collect();
    files.push(("slots/beta/Image".to_string(), pattern(10, 9)));
    let boot = fat_volume(2 << 20, &files);
    gpt_disk(&[
        Part {
            name: "boot",
            first_lba: 2048,
            bytes: &boot,
        },
        Part {
            name: "data",
            first_lba: 2048 + 4096,
            bytes: DATA_FIXTURE,
        },
    ])
}

/// The bring-up form's disk: `rootfs` (the boot files) then `data` (the slot).
pub fn bringup_disk() -> Vec<u8> {
    gpt_disk(&[
        Part {
            name: "rootfs",
            first_lba: 2048,
            bytes: ROOTFS_FIXTURE,
        },
        Part {
            name: "data",
            first_lba: 2048 + 512,
            bytes: DATA_FIXTURE,
        },
    ])
}
