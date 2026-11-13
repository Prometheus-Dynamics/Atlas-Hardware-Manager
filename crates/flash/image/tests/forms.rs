//! Both image forms, from an .img.xz and from a plain .img: the slot's verity
//! table, its file and its boot files come back byte for byte, and the scratch
//! directory holds only what the read keeps.

mod common;

use std::io::Read;

use atlas_image::{Form, ImageError, Options, open};
use common::*;

fn read_all(mut reader: Box<dyn Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    bytes
}

fn check_bring_up(path: &std::path::Path, scratch: &std::path::Path) {
    let image = open(path, scratch, &["alpha"], &Options::default()).expect("open");
    assert_eq!(image.form(), Form::BringUp);
    let names: Vec<&str> = image.partitions().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["rootfs", "data"]);

    let slot = image.slot("alpha").expect("slot");
    assert_eq!(slot.verity_table().expect("table"), verity_text());
    assert_eq!(slot.rootfs_len().expect("len"), slot_file().len() as u64);
    assert_eq!(read_all(slot.rootfs().expect("rootfs")), slot_file());
    // Read again: the bundle writer reads the slot twice.
    assert_eq!(read_all(slot.rootfs().expect("rootfs")), slot_file());

    let files = slot.boot_files().expect("boot files");
    let listed: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(listed, ["Image", "board.dtb", "extlinux/extlinux.conf"]);
    for (path, bytes) in bringup_boot() {
        assert_eq!(
            read_all(slot.boot_file(path).expect("boot file")),
            bytes,
            "{path}"
        );
    }
    let size = files.iter().find(|f| f.path == "Image").map(|f| f.size);
    assert_eq!(size, Some(40_000));
}

fn check_real(path: &std::path::Path, scratch: &std::path::Path) {
    let image = open(path, scratch, &["alpha"], &Options::default()).expect("open");
    assert_eq!(image.form(), Form::Real);
    let slot = image.slot("alpha").expect("slot");
    assert_eq!(slot.verity_table().expect("table"), verity_text());
    assert_eq!(read_all(slot.rootfs().expect("rootfs")), slot_file());

    let files = slot.boot_files().expect("boot files");
    let listed: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        listed,
        ["Image", "board.dtb", "cmdline.txt", "initramfs.cpio.gz"]
    );
    for (path, bytes) in real_boot() {
        assert_eq!(
            read_all(slot.boot_file(path).expect("boot file")),
            bytes,
            "{path}"
        );
    }
    // Slot beta is on the boot partition but was not asked for.
    assert!(image.slot("beta").is_err());
}

#[test]
fn bring_up_form_from_xz_gives_the_parts_byte_for_byte() {
    let dir = TestDir::new("bringup-xz");
    let scratch = TestDir::new("bringup-xz-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&bringup_disk()));
    check_bring_up(&path, scratch.path());
}

#[test]
fn bring_up_form_from_plain_img_gives_the_same_parts() {
    let dir = TestDir::new("bringup-img");
    let scratch = TestDir::new("bringup-img-scratch");
    let path = dir.write("first-flash.img", &bringup_disk());
    check_bring_up(&path, scratch.path());
}

#[test]
fn real_form_from_xz_gives_the_parts_byte_for_byte() {
    let dir = TestDir::new("real-xz");
    let scratch = TestDir::new("real-xz-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&real_disk()));
    check_real(&path, scratch.path());
}

#[test]
fn real_form_from_plain_img_gives_the_same_parts() {
    let dir = TestDir::new("real-img");
    let scratch = TestDir::new("real-img-scratch");
    let path = dir.write("first-flash.img", &real_disk());
    check_real(&path, scratch.path());
}

#[test]
fn the_boot_spool_is_gone_and_the_data_spool_is_kept_while_the_image_lives() {
    let dir = TestDir::new("spool-kept");
    let scratch = TestDir::new("spool-kept-scratch");
    let path = dir.write("first-flash.img.xz", &xz(&real_disk()));
    let image = open(&path, scratch.path(), &["alpha"], &Options::default()).expect("open");
    let inside = std::fs::read_dir(image.scratch_dir())
        .expect("scratch")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        inside.iter().any(|name| name == "partition-1.img"),
        "{inside:?}"
    );
    assert!(
        !inside.iter().any(|name| name == "partition-0.img"),
        "{inside:?}"
    );
    assert!(inside.iter().any(|name| name == "boot"), "{inside:?}");
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
    let whole = xz(&real_disk());
    // Cut in the data partition, after the boot partition was spooled and removed.
    let path = dir.write("cut.img.xz", &whole[..whole.len() - 40]);
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
fn a_data_partition_before_the_boot_partition_is_refused() {
    let dir = TestDir::new("order");
    let scratch = TestDir::new("order-scratch");
    let boot = fat_volume(
        2 << 20,
        &[("slots/alpha/Image".to_string(), pattern(10, 1))],
    );
    let disk = gpt_disk(&[
        Part {
            name: "data",
            first_lba: 2048,
            bytes: DATA_FIXTURE,
        },
        Part {
            name: "boot",
            first_lba: 2048 + 512,
            bytes: &boot,
        },
    ]);
    let path = dir.write("order.img", &disk);
    let error = open(&path, scratch.path(), &["alpha"], &Options::default()).unwrap_err();
    assert!(matches!(error, ImageError::Layout(_)), "{error}");
    assert!(scratch.entries().is_empty());
}

#[test]
fn a_slot_the_image_does_not_have_is_refused_at_open() {
    let dir = TestDir::new("missing-slot");
    let scratch = TestDir::new("missing-slot-scratch");
    let path = dir.write("first-flash.img", &bringup_disk());
    let error = open(&path, scratch.path(), &["gamma"], &Options::default()).unwrap_err();
    assert!(matches!(error, ImageError::Missing(_)), "{error}");
}

#[test]
fn a_bad_slot_id_is_refused_before_the_image_is_read() {
    let dir = TestDir::new("bad-id");
    let scratch = TestDir::new("bad-id-scratch");
    let path = dir.write("first-flash.img", &bringup_disk());
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
fn the_spool_cap_refuses_a_partition_before_spooling_it() {
    let dir = TestDir::new("cap-spool");
    let scratch = TestDir::new("cap-spool-scratch");
    let path = dir.write("first-flash.img", &real_disk());
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
    let path = dir.write("first-flash.img", &real_disk());
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
