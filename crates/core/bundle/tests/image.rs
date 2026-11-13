//! The bundle from a first-flash image, through `atlas-image`.
//!
//! The image is the reader's own fixture (`crates/flash/image/tests/fixtures`,
//! made by its make-fixtures.py), wrapped in a GPT the way the reader's tests
//! wrap it (a copy of its small GPT writer is below: the reader's helpers are
//! test-private). The fixture's slot `alpha` has four data blocks and one tree
//! block; its manifest's root hash was made with the salt below.
//!
//! The one thing the image cannot give is the verity table (salt and block
//! count), so the writer test supplies one from the test's own knowledge of
//! the fixture, and says so. The one-call path is tested to stop at the named
//! error.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use atlas_bundle::{
    BLOCK, BootFile, BundleError, ImageSource, SlotMeta, SlotSource, bundle_from_image,
    check_stream_digest, inspect_bundle, verify_slot, write_bundle,
};
use atlas_release::{PublicKey, public_key_for};
use sha2::{Digest, Sha256};

const DATA_FIXTURE: &[u8] = include_bytes!("../../../flash/image/tests/fixtures/data.ext4");
/// The salt the fixture's verity roots were made with (its make-fixtures.py).
const SALT: [u8; 16] = [0x5A; 16];

fn pattern(n: usize, seed: u64) -> Vec<u8> {
    (0..n as u64)
        .map(|i| ((i * 7 + seed * 13) % 251) as u8)
        .collect()
}

/// Slot `alpha`'s file as the fixture holds it: four data blocks, one tree block.
fn alpha_file() -> Vec<u8> {
    let mut bytes = pattern(4 * BLOCK, 1);
    bytes.extend_from_slice(&pattern(BLOCK, 2));
    bytes
}

/// Slot `alpha`'s boot files, by path, as the fixture lists them.
fn alpha_boot() -> Vec<(&'static str, Vec<u8>)> {
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn throwaway(label: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(label.as_bytes());
    hasher.finalize().into()
}

fn keys_of(secret: &[u8; 32]) -> Vec<PublicKey> {
    vec![public_key_for(secret)]
}

fn meta() -> SlotMeta {
    SlotMeta {
        product: "example".into(),
        version: "2026.1.0".into(),
        board: "example-board".into(),
        ..SlotMeta::default()
    }
}

/// A test directory under the target's tmp dir, removed on drop.
struct TestDir(PathBuf);

static NEXT: AtomicU64 = AtomicU64::new(0);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "atlas-bundle-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

/// A disk with one GPT partition, `data`, holding `bytes` at LBA 2048. A copy
/// of the reader's test writer: a protective MBR, the primary GPT, no backup.
fn gpt_disk_with_data(bytes: &[u8]) -> Vec<u8> {
    let first_lba = 2048u64;
    let end = (first_lba * 512) as usize + bytes.len();
    let mut disk = vec![0u8; end];
    disk[510] = 0x55;
    disk[511] = 0xAA;

    let mut entries = vec![0u8; 128 * 128];
    let entry = &mut entries[0..128];
    entry[0..16].copy_from_slice(&[0x11; 16]);
    entry[16..32].copy_from_slice(&[1; 16]);
    let last = first_lba + (bytes.len() as u64).div_ceil(512) - 1;
    entry[32..40].copy_from_slice(&first_lba.to_le_bytes());
    entry[40..48].copy_from_slice(&last.to_le_bytes());
    let (units, _) = entry[56..128].as_chunks_mut::<2>();
    for (unit, value) in units.iter_mut().zip("data".encode_utf16()) {
        *unit = value.to_le_bytes();
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
    disk[(first_lba * 512) as usize..].copy_from_slice(bytes);
    disk
}

/// Writes the fixture's image (a plain `.img`) into `dir`.
fn image_file(dir: &TestDir) -> PathBuf {
    let path = dir.path().join("first-flash.img");
    fs::write(&path, gpt_disk_with_data(DATA_FIXTURE)).expect("write image");
    path
}

/// The image's slot `alpha`, with the verity table the test supplies: the
/// salt and block count the fixture was made with, and the root its manifest
/// carries. The image gives the rest.
struct WithTable<'a> {
    inner: ImageSource<'a>,
    table: String,
}

impl SlotSource for WithTable<'_> {
    fn verity_table(&self) -> Result<String, BundleError> {
        Ok(self.table.clone())
    }
    fn rootfs(&self) -> Result<Box<dyn Read + '_>, BundleError> {
        self.inner.rootfs()
    }
    fn boot_files(&self) -> Result<Vec<BootFile>, BundleError> {
        self.inner.boot_files()
    }
    fn boot_file(&self, path: &str) -> Result<Box<dyn Read + '_>, BundleError> {
        self.inner.boot_file(path)
    }
}

/// The members of a bundle: name and contents.
fn members(bundle: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = tar::Archive::new(bundle);
    archive
        .entries()
        .expect("entries")
        .map(|entry| {
            let mut entry = entry.expect("entry");
            let name = entry.path().expect("path").to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("read");
            (name, bytes)
        })
        .collect()
}

#[test]
fn a_bundle_from_the_image_matches_the_slots_parts_member_by_member() {
    let dir = TestDir::new("parts");
    let image_path = image_file(&dir);
    let image = atlas_image::open(
        &image_path,
        dir.path(),
        &["alpha"],
        &atlas_image::Options::default(),
    )
    .expect("the reader opens the fixture");
    let slot = image.slot("alpha").expect("alpha");
    let source = WithTable {
        table: format!(
            "hash_type 1\nalgorithm sha256\nroot_hash {}\nsalt {}\ndata_blocks 4\n\
             data_block_size 4096\nhash_block_size 4096\nhash_offset 16384\n",
            slot.root_hash(),
            hex(&SALT)
        ),
        inner: ImageSource::new(slot),
    };
    source
        .inner
        .check_image_digest()
        .expect("the slot file is what its manifest says");

    let secret = throwaway("parts");
    let mut bundle = Vec::new();
    let manifest = write_bundle(&source, "alpha", &meta(), &secret, &mut bundle).expect("writes");

    let all = members(&bundle);
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "slot.json",
            "slot.sig",
            "rootfs.erofs",
            "boot/Image",
            "boot/board.dtb",
            "boot/cmdline.txt",
            "boot/initramfs.cpio.gz",
            "boot/overlays/one.dtbo",
        ],
        "the image's slot.json is not a boot file of the bundle"
    );
    let get = |name: &str| &all.iter().find(|(n, _)| n == name).expect(name).1;

    // The slot file, byte for byte, and its digest against the image manifest.
    assert_eq!(get("rootfs.erofs"), &alpha_file());
    assert_eq!(manifest.image.sha256, hex(&Sha256::digest(alpha_file())));
    assert_eq!(manifest.image.size, alpha_file().len() as u64);
    // Each boot file, byte for byte.
    for (path, bytes) in alpha_boot() {
        assert_eq!(get(&format!("boot/{path}")), &bytes, "boot/{path}");
    }
    // slot.json and its signature.
    let json = get("slot.json");
    let sig = get("slot.sig");
    assert_eq!(sig.len(), 64);
    assert_eq!(
        verify_slot(json, sig, &keys_of(&secret)).expect("verifies"),
        0
    );
    let inspected = inspect_bundle(&bundle[..], &keys_of(&secret)).expect("inspects");
    assert_eq!(inspected.board, "example-board");
    assert_eq!(inspected.manifest.verity.root_hash, slot_root());
    assert_eq!(inspected.manifest.boot.files.len(), 5);
    assert!(!inspected.manifest.boot.files.contains_key("slot.json"));

    // One changed byte breaks the signature, and the inspector refuses it.
    let mut changed = json.clone();
    changed[20] ^= 1;
    assert!(verify_slot(&changed, sig, &keys_of(&secret)).is_err());
}

fn slot_root() -> String {
    // The fixture's manifest root for alpha, as the reader exposes it.
    let dir = TestDir::new("root");
    let image_path = image_file(&dir);
    let image = atlas_image::open(
        &image_path,
        dir.path(),
        &["alpha"],
        &atlas_image::Options::default(),
    )
    .expect("open");
    image.slot("alpha").expect("alpha").root_hash().to_string()
}

#[test]
fn the_one_call_stops_at_the_named_verity_error_and_leaves_no_scratch() {
    let dir = TestDir::new("oncall");
    let image_path = image_file(&dir);
    let scratch = dir.path().join("scratch");
    fs::create_dir_all(&scratch).expect("scratch parent");
    let mut out = Vec::new();
    let error = bundle_from_image(
        &image_path,
        &scratch,
        "alpha",
        &meta(),
        &throwaway("oncall"),
        &mut out,
    )
    .expect_err("no table can be made from an image yet");
    assert!(matches!(error, BundleError::NoVerityTable(_)), "{error:?}");
    assert!(error.to_string().contains("alpha"), "{error}");
    assert!(out.is_empty(), "nothing is written before the table step");
    assert!(
        fs::read_dir(&scratch).expect("scratch").next().is_none(),
        "the scratch spool is removed"
    );
}

#[test]
fn a_missing_slot_is_refused_by_the_reader_with_its_message() {
    let dir = TestDir::new("missing");
    let image_path = image_file(&dir);
    let mut out = Vec::new();
    let error = bundle_from_image(
        &image_path,
        dir.path(),
        "gamma",
        &meta(),
        &throwaway("missing"),
        &mut out,
    )
    .expect_err("refused");
    assert!(matches!(error, BundleError::Image(_)), "{error:?}");
    assert!(error.to_string().contains("gamma"), "{error}");
}

#[test]
fn a_slot_file_whose_digest_is_not_the_manifests_is_refused() {
    let good = alpha_file();
    let mut bad = good.clone();
    bad[0] ^= 1;
    let sha = hex(&Sha256::digest(&good));
    check_stream_digest(&mut &good[..], good.len() as u64, &sha).expect("the right bytes pass");
    let error = check_stream_digest(&mut &bad[..], bad.len() as u64, &sha).expect_err("refused");
    assert!(
        matches!(error, BundleError::ImageDigest { .. }),
        "{error:?}"
    );
    // A length that differs is refused too, even with the right bytes.
    assert!(check_stream_digest(&mut &good[..], good.len() as u64 - 1, &sha).is_err());
}
