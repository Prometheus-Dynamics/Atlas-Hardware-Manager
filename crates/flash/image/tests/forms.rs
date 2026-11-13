//! Slots read from an .img.xz and from a plain .img: the file, the manifest and
//! the boot files come back byte for byte, the root hash is the manifest's, and
//! the scratch directory holds only the data spool while the image lives.

mod common;

use std::io::Read;

use atlas_image::{ImageError, Options, open};
use common::*;
use sha2::{Digest, Sha256};

fn read_all(mut reader: Box<dyn Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    bytes
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn check_alpha(path: &std::path::Path, scratch: &std::path::Path) {
    let image = open(path, scratch, &["alpha"], &Options::default()).expect("open");
    let names: Vec<&str> = image.partitions().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["data"]);

    let slot = image.slot("alpha").expect("slot");
    assert_eq!(
        slot.root_hash(),
        alpha_root(),
        "the manifest's root is the tree's root"
    );
    assert_eq!(slot.image_size(), alpha_file().len() as u64);
    // The four verity values, as the manifest gives them: salt, four data blocks,
    // the tree after them.
    let verity = slot.verity();
    assert_eq!(verity.root_hash, alpha_root());
    assert_eq!(verity.salt, SALT);
    assert_eq!(verity.data_blocks, 4);
    assert_eq!(verity.hash_offset, 4 * BLOCK as u64);
    assert_eq!(read_all(slot.rootfs().expect("rootfs")), alpha_file());
    // Read again: a bundle writer reads the slot twice.
    assert_eq!(read_all(slot.rootfs().expect("rootfs")), alpha_file());

    let listed: Vec<String> = slot.boot_files().into_iter().map(|f| f.path).collect();
    assert_eq!(
        listed,
        [
            "Image",
            "board.dtb",
            "cmdline.txt",
            "initramfs.cpio.gz",
            "overlays/one.dtbo",
            "slot.json"
        ]
    );
    for (path, bytes) in alpha_boot() {
        assert_eq!(
            read_all(slot.boot_file(path).expect("boot file")),
            bytes,
            "{path}"
        );
    }
    // slot.json in the boot list is the manifest exactly as the image holds it.
    assert_eq!(
        read_all(slot.boot_file("slot.json").expect("slot.json")),
        slot.manifest_json().as_bytes()
    );
    for entry in slot.boot_files() {
        let bytes = read_all(slot.boot_file(&entry.path).expect("boot file"));
        assert_eq!(entry.size, bytes.len() as u64, "{}", entry.path);
        assert_eq!(entry.sha256, hex(&bytes), "{}", entry.path);
    }
    assert!(image.slot("beta").is_err(), "beta was not asked for");
}

#[test]
fn a_slot_from_an_xz_image_gives_its_parts_byte_for_byte() {
    let dir = TestDir::new("alpha-xz");
    let scratch = TestDir::new("alpha-xz-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&image_disk()));
    check_alpha(&path, scratch.path());
}

#[test]
fn a_slot_from_a_plain_img_gives_the_same_parts() {
    let dir = TestDir::new("alpha-img");
    let scratch = TestDir::new("alpha-img-scratch");
    let path = dir.write("first-flash.img", &image_disk());
    check_alpha(&path, scratch.path());
}

#[test]
fn two_slots_from_one_read_do_not_mix() {
    let dir = TestDir::new("two");
    let scratch = TestDir::new("two-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&image_disk()));
    let image = open(
        &path,
        scratch.path(),
        &["alpha", "beta"],
        &Options::default(),
    )
    .expect("open");
    let beta = image.slot("beta").expect("beta");
    assert_eq!(beta.root_hash(), beta_root());
    assert_eq!(read_all(beta.rootfs().expect("rootfs")), beta_file());
    let listed: Vec<String> = beta.boot_files().into_iter().map(|f| f.path).collect();
    assert_eq!(listed, ["cmdline.txt", "slot.json"]);
    assert_eq!(
        read_all(beta.boot_file("cmdline.txt").expect("cmdline")),
        beta_boot()[0].1
    );
    let alpha = image.slot("alpha").expect("alpha");
    assert_eq!(alpha.root_hash(), alpha_root());
}

#[test]
fn only_the_data_spool_is_kept_while_the_image_lives() {
    let dir = TestDir::new("spool-kept");
    let scratch = TestDir::new("spool-kept-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&image_disk()));
    let image = open(&path, scratch.path(), &["alpha"], &Options::default()).expect("open");
    let inside: Vec<String> = std::fs::read_dir(image.scratch_dir())
        .expect("scratch")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(inside, ["data.img"]);
    drop(image);
    assert!(
        scratch.entries().is_empty(),
        "left behind: {:?}",
        scratch.entries()
    );
}

#[test]
fn a_read_that_fails_part_way_leaves_no_scratch_behind() {
    let dir = TestDir::new("fail-clean");
    let scratch = TestDir::new("fail-clean-scratch");
    let whole = xz(&image_disk());
    let path = dir.write("cut.img.xz", &whole[..whole.len() / 2]);
    let error = open(&path, scratch.path(), &["alpha"], &Options::default()).unwrap_err();
    assert!(
        matches!(error, ImageError::Truncated(_) | ImageError::Decode(_)),
        "{error}"
    );
    assert!(
        scratch.entries().is_empty(),
        "left behind: {:?}",
        scratch.entries()
    );
}

#[test]
fn a_slot_the_image_does_not_have_is_refused_at_open() {
    let dir = TestDir::new("missing-slot");
    let scratch = TestDir::new("missing-slot-scratch");
    let path = dir.write("first-flash.img", &image_disk());
    let error = open(&path, scratch.path(), &["gamma"], &Options::default()).unwrap_err();
    assert!(matches!(error, ImageError::Missing(_)), "{error}");
    assert!(scratch.entries().is_empty());
}

#[test]
fn a_bad_slot_id_is_refused_before_anything_is_made() {
    let dir = TestDir::new("bad-id");
    let scratch = TestDir::new("bad-id-scratch");
    let path = dir.write("first-flash.img", &image_disk());
    for id in ["../alpha", "", "a/b", ".."] {
        let error = open(&path, scratch.path(), &[id], &Options::default()).unwrap_err();
        assert!(
            matches!(error, ImageError::InvalidName(_)),
            "{id:?}: {error}"
        );
    }
    assert!(scratch.entries().is_empty());
}

#[test]
fn an_image_without_a_data_partition_is_refused() {
    let dir = TestDir::new("no-data");
    let scratch = TestDir::new("no-data-scratch");
    let disk = gpt_disk(&[Part {
        name: "rootfs",
        first_lba: 2048,
        bytes: DATA_FIXTURE,
    }]);
    let path = dir.write("rootfs-only.img", &disk);
    let error = open(&path, scratch.path(), &["alpha"], &Options::default()).unwrap_err();
    assert!(matches!(error, ImageError::Layout(_)), "{error}");
    assert!(scratch.entries().is_empty());
}

#[test]
fn a_boot_file_that_is_not_its_name_is_refused() {
    let dir = TestDir::new("tampered");
    let scratch = TestDir::new("tampered-scratch");
    // The Image blob's blocks in the fixture (debugfs: blocks 71 to 110, 1 KiB
    // each). The pattern repeats, so a search would find other copies; the offset
    // is checked against the expected bytes before one is flipped.
    let blob_at = 71 * 1024;
    assert_eq!(
        &DATA_FIXTURE[blob_at..blob_at + 64],
        &pattern(40_000, 3)[..64],
        "the fixture's Image blob is not where the test expects"
    );
    let mut disk = image_disk();
    disk[2048 * 512 + blob_at + 10] ^= 0x01;
    let path = dir.write("tampered.img", &disk);
    let error = open(&path, scratch.path(), &["alpha"], &Options::default()).unwrap_err();
    assert!(matches!(error, ImageError::Malformed(_)), "{error}");
    assert!(scratch.entries().is_empty());
}

#[test]
fn the_spool_cap_refuses_the_data_partition_before_spooling_it() {
    let dir = TestDir::new("cap-spool");
    let scratch = TestDir::new("cap-spool-scratch");
    let path = dir.write("first-flash.img", &image_disk());
    let options = Options {
        max_spool_bytes: 64 * 1024,
        ..Options::default()
    };
    let error = open(&path, scratch.path(), &["alpha"], &options).unwrap_err();
    assert!(matches!(error, ImageError::Limit(_)), "{error}");
    assert!(scratch.entries().is_empty());
}

#[test]
fn the_boot_file_caps_refuse_a_big_file_and_a_big_total() {
    let dir = TestDir::new("cap-boot");
    let scratch = TestDir::new("cap-boot-scratch");
    let path = dir.write("first-flash.img", &image_disk());
    let per_file = Options {
        max_file_bytes: 1000,
        ..Options::default()
    };
    let error = open(&path, scratch.path(), &["alpha"], &per_file).unwrap_err();
    assert!(matches!(error, ImageError::Limit(_)), "{error}");
    let total = Options {
        max_boot_bytes: 20_000,
        ..Options::default()
    };
    let error = open(&path, scratch.path(), &["alpha"], &total).unwrap_err();
    assert!(matches!(error, ImageError::Limit(_)), "{error}");
    assert!(scratch.entries().is_empty());
}
