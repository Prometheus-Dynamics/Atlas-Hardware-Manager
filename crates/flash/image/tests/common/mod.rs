//! Test helpers: the deterministic contents of the fixture's slots, a GPT writer,
//! the xz wrapper, and a test directory that removes itself.

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use atlas_bundle::root_hash;

pub const DATA_FIXTURE: &[u8] = include_bytes!("../fixtures/data.ext4");

/// The salt the fixture's verity roots were made with (make-fixtures.py): 32 bytes.
pub const SALT: [u8; 32] = [0x5A; 32];
pub const BLOCK: usize = 4096;

/// The bytes the fixture was made from (the same formula as make-fixtures.py).
pub fn pattern(n: usize, seed: u64) -> Vec<u8> {
    (0..n as u64)
        .map(|i| ((i * 7 + seed * 13) % 251) as u8)
        .collect()
}

/// Slot `alpha`'s file: four data blocks, then one tree block.
pub fn alpha_file() -> Vec<u8> {
    let mut bytes = pattern(4 * BLOCK, 1);
    bytes.extend_from_slice(&pattern(BLOCK, 2));
    bytes
}

/// Slot `beta`'s file: two blocks (one data, one tree).
pub fn beta_file() -> Vec<u8> {
    // One data block and no tree.
    pattern(BLOCK, 7)
}

/// Slot `alpha`'s boot files, by path (the manifest's `boot.files` and the
/// `.boot` list's content, except `slot.json`, which is the manifest itself).
pub fn alpha_boot() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Image", pattern(40_000, 3)),
        ("board.dtb", pattern(1234, 4)),
        ("initramfs.cpio.gz", pattern(9000, 5)),
        (
            "cmdline.txt",
            b"console=tty1 root=PARTUUID=abc panic=5\n".to_vec(),
        ),
        ("overlays/one.dtbo", pattern(777, 6)),
    ]
}

/// Slot `beta`'s boot files (its `cmdline.txt` is not alpha's).
pub fn beta_boot() -> Vec<(&'static str, Vec<u8>)> {
    vec![("cmdline.txt", b"console=tty2 panic=5\n".to_vec())]
}

/// The verity root of slot `alpha`, computed by the bundle crate's hasher. The
/// fixture's manifest carries the same value (checked by the tests).
pub fn alpha_root() -> String {
    root_hash(&SALT, &alpha_file()[..4 * BLOCK]).expect("root")
}

/// The verity root of slot `beta`.
pub fn beta_root() -> String {
    root_hash(&SALT, &beta_file()).expect("root")
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
/// LBA 2) and the partitions at their sectors. No backup table: the reader does
/// not need one.
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

/// The xz file of `raw`, as the image's build writes it: the `xz` program
/// (liblzma, the same library as Gaia's xz writer) with its default CRC-64 block
/// check. The pure-Rust encoder in lzma-rs writes no check at all, so it cannot
/// show that a damaged stream is refused.
pub fn xz(raw: &[u8]) -> Vec<u8> {
    xz_with(raw, &["--check=crc64"])
}

/// [`xz`] with the given options (a preset such as `-7`, a block size).
pub fn xz_with(raw: &[u8], options: &[&str]) -> Vec<u8> {
    let mut child = Command::new("xz")
        .args(options)
        .args(["--compress", "--stdout"])
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

/// The image: a GPT with one partition, `data`, the fixture.
pub fn image_disk() -> Vec<u8> {
    gpt_disk(&[Part {
        name: "data",
        first_lba: 2048,
        bytes: DATA_FIXTURE,
    }])
}

/// Bytes with no repeats the compressor can use: a xorshift stream, so a large
/// one stays large once compressed.
pub fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// A data partition (ext4, 512 KiB, label `data`) holding `files` (paths inside
/// it, relative to the root), made by `mke2fs -d` as the fixture is: no mount, no
/// root. The files are written into a staging directory first.
pub fn ext4_data(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let dir = TestDir::new("ext4-stage");
    let stage = dir.path().join("stage");
    for (path, bytes) in files {
        let full = stage.join(path);
        fs::create_dir_all(full.parent().expect("parent")).expect("stage dir");
        fs::write(&full, bytes).expect("stage file");
    }
    let out = dir.path().join("data.ext4");
    fs::write(&out, vec![0u8; 512 * 1024]).expect("image file");
    let status = Command::new("mke2fs")
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
            "data",
            "-d",
        ])
        .arg(&stage)
        .arg(&out)
        .status()
        .expect("mke2fs is needed to make the test images");
    assert!(status.success(), "mke2fs failed");
    fs::read(&out).expect("read image")
}

/// The data partition's slot manifest as the board check reads it: a schema 3
/// manifest with `board` set to `board` (or left out when `None`).
pub fn slot_json(id: &str, board: Option<&str>) -> Vec<u8> {
    let board = board.map_or(String::new(), |b| format!("\"board\":\"{b}\","));
    format!("{{\"schema\":3,\"id\":\"{id}\",\"product\":\"example\",\"version\":\"1\",{board}\"image\":{{\"size\":4096,\"sha256\":\"{}\"}}}}", "0".repeat(64)).into_bytes()
}
