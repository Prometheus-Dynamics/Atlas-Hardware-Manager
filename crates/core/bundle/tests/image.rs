//! The bundle from a first-flash image, through `atlas-image`.
//!
//! The image is the reader's own fixture (`crates/flash/image/tests/fixtures`,
//! made by its make-fixtures.py), wrapped in a GPT the way the reader's tests
//! wrap it (a copy of its small GPT writer is below: the reader's helpers are
//! test-private). The fixture's slot `alpha` has four data blocks and one tree
//! block; its manifest's root hash was made with the salt below.
//!
//! The bundle is rebuilt in one call (`bundle_from_image`) from what the image
//! keeps: its manifest and, for alpha, the kept signature (made with the
//! fixture's throwaway seed). Nothing is signed here; the parts are checked
//! against the image's member by member.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use atlas_bundle::{
    BLOCK, BundleError, RECORD_LEN, bundle_from_image, inspect_bundle, verify_slot,
};
use atlas_release::public_key_for;
use sha2::{Digest, Sha256};

const DATA_FIXTURE: &[u8] = include_bytes!("../../../flash/image/tests/fixtures/data.ext4");

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

/// The fixture's throwaway signing seed (make-fixtures.py `SIGNING_SEED`): alpha's kept slot.sig is
/// made with it.
const FIXTURE_SEED: [u8; 32] = [0x11; 32];

fn make(dir: &TestDir, slot: &str) -> Result<(atlas_bundle::ImageBundle, Vec<u8>), BundleError> {
    let image_path = image_file(dir);
    let scratch = dir.path().join("scratch");
    fs::create_dir_all(&scratch).expect("scratch parent");
    let mut bundle = Vec::new();
    let made = bundle_from_image(&image_path, &scratch, slot, &mut bundle)?;
    assert!(
        fs::read_dir(&scratch).expect("scratch").next().is_none(),
        "nothing is left of the scratch spool"
    );
    Ok((made, bundle))
}

/// The image's own bytes for a file on its data partition.
fn image_file_bytes(dir: &TestDir, path: &str) -> Option<Vec<u8>> {
    let image = atlas_image::open(
        &image_file(dir),
        dir.path(),
        &["alpha", "beta"],
        &atlas_image::Options::default(),
    )
    .expect("open");
    let (id, kind) = path.rsplit_once('.').expect("id.kind");
    let slot = image.slot(id).expect("slot");
    match kind {
        "json" => Some(slot.manifest_json().as_bytes().to_vec()),
        "sig" => slot.signature().map(<[u8]>::to_vec),
        _ => panic!("{path}"),
    }
}

#[test]
fn a_signed_slot_s_bundle_is_its_kept_manifest_and_signature_and_verifies_with_no_key_on_the_computer()
 {
    let dir = TestDir::new("signed");
    let (made, bundle) = make(&dir, "alpha").expect("one call makes the bundle");
    assert!(made.signed);
    assert_eq!(made.manifest.verity.root_hash, slot_root());

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
        "Lemnos's order; the boot partition's slot.json is not a boot member"
    );
    let get = |name: &str| &all.iter().find(|(n, _)| n == name).expect(name).1;

    // slot.json and slot.sig are the image's kept bytes, unchanged: nothing was written or signed here.
    assert_eq!(
        get("slot.json"),
        &image_file_bytes(&dir, "alpha.json").expect("json")
    );
    assert_eq!(
        get("slot.sig"),
        &image_file_bytes(&dir, "alpha.sig").expect("kept sig")
    );
    assert_eq!(get("slot.sig").len(), RECORD_LEN);
    // And they verify as the bundle did, under the key that signed it, and under no other.
    assert_eq!(
        verify_slot(
            get("slot.json"),
            get("slot.sig"),
            &[public_key_for(&FIXTURE_SEED)]
        )
        .expect("verifies"),
        0
    );
    assert!(
        verify_slot(
            get("slot.json"),
            get("slot.sig"),
            &[public_key_for(&[0x22; 32])]
        )
        .is_err()
    );
    let inspected =
        inspect_bundle(&bundle[..], &[public_key_for(&FIXTURE_SEED)]).expect("inspects");
    assert_eq!(inspected.board, "example-board");
    assert_eq!(inspected.manifest.boot.files.len(), 5);

    // The slot file and each boot file, byte for byte.
    assert_eq!(get("rootfs.erofs"), &alpha_file());
    assert_eq!(
        made.manifest.image.sha256,
        hex(&Sha256::digest(alpha_file()))
    );
    for (path, bytes) in alpha_boot() {
        assert_eq!(get(&format!("boot/{path}")), &bytes, "boot/{path}");
    }
}

#[test]
fn an_unsigned_slot_gives_an_unsigned_bundle_never_one_signed_on_the_computer() {
    let dir = TestDir::new("unsigned");
    assert_eq!(image_file_bytes(&dir, "beta.sig"), None);
    let (made, bundle) = make(&dir, "beta").expect("made");
    assert!(!made.signed);
    let all = members(&bundle);
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["slot.json", "rootfs.erofs", "boot/cmdline.txt"]);
    assert_eq!(all[0].1, image_file_bytes(&dir, "beta.json").expect("json"));
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
fn a_missing_slot_is_refused_by_the_reader_with_its_message() {
    let dir = TestDir::new("missing");
    let error = make(&dir, "gamma").expect_err("refused");
    assert!(matches!(error, BundleError::Image(_)), "{error:?}");
    assert!(error.to_string().contains("gamma"), "{error}");
}
