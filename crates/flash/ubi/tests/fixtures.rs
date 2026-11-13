//! The image builder's own UBI fixtures, read by this crate's reader. The
//! expectations come from the inputs the builder's tests give for each
//! fixture (volume names, sizes, PEB sizes, sequence numbers), not from this
//! crate's writer or encoders. A disagreement is a failure to report, not a
//! reason to change the expectation.

use atlas_ubi::headers::{HEADER_SIZE, PebLayout, VolumeKind};
use atlas_ubi::image::{ImageError, Volume, read_image};
use std::io::Cursor;

fn fixture(name: &str) -> Vec<u8> {
    let path: std::path::PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The builder's rootfs input: `(i * 7) % 251` over 20 000 bytes.
fn rootfs() -> Vec<u8> {
    (0..20_000u32)
        .map(|i| (i.wrapping_mul(7) % 251) as u8)
        .collect()
}

fn read(bytes: &[u8], peb_size: u32) -> Result<atlas_ubi::image::Image, ImageError> {
    read_image(&mut Cursor::new(bytes), peb_size)
}

fn volume<'a>(image: &'a atlas_ubi::image::Image, name: &str) -> &'a Volume {
    image
        .volume(name)
        .unwrap_or_else(|| panic!("no volume {name} in {:?}", image.volumes))
}

#[test]
fn nand_16k_is_read_as_the_builder_wrote_it() {
    let bytes = fixture("nand-16k.ubi");
    let image = read(&bytes, 16 * 1024).expect("reads");
    assert_eq!(
        image.layout,
        PebLayout {
            peb_size: 16 * 1024,
            vid_header_offset: 512,
            data_offset: 1024
        }
    );
    assert_eq!((image.pebs, image.image_seq), (6, 0x1234_5678));
    // Inputs: rootfs_a 4 LEBs reserved, 20 000 bytes; kernel static 5 000
    // bytes; config 4 000 reserved, 3 000 bytes; data 1 autoresize.
    let rootfs_a = volume(&image, "rootfs_a");
    assert_eq!(
        (rootfs_a.id, rootfs_a.kind, rootfs_a.reserved_lebs),
        (0, VolumeKind::Dynamic, 4)
    );
    assert_eq!((rootfs_a.image_lebs, rootfs_a.first_peb), (2, Some(2)));
    let kernel = volume(&image, "kernel");
    assert_eq!((kernel.id, kernel.kind), (1, VolumeKind::Static));
    assert_eq!((kernel.image_lebs, kernel.first_peb), (1, Some(4)));
    assert_eq!(kernel.static_bytes, Some(5_000));
    let config = volume(&image, "config");
    assert_eq!((config.id, config.kind), (2, VolumeKind::Dynamic));
    assert_eq!((config.image_lebs, config.first_peb), (1, Some(5)));
    let data = volume(&image, "data");
    assert_eq!((data.id, data.reserved_lebs), (3, 1));
    assert!(data.autoresize);
    assert_eq!(data.image_lebs, 0);
    assert_eq!(image.reserved_pebs(), 2 + 2 + 4 + 1 + 1 + 1);

    // The first rootfs LEB's bytes sit at PEB 2's data offset, as given.
    assert_eq!(&bytes[2 * 16_384 + 1024..][..15_360], &rootfs()[..15_360]);
    // The kernel's static LEB holds 5 000 bytes of (i % 13), its CRC checked
    // by the reader.
}

#[test]
fn nor_4k_is_read_as_the_builder_wrote_it() {
    let bytes = fixture("nor-4k.ubi");
    let image = read(&bytes, 4096).expect("reads");
    assert_eq!(
        image.layout,
        PebLayout {
            peb_size: 4096,
            vid_header_offset: 64,
            data_offset: 128
        }
    );
    assert_eq!((image.pebs, image.image_seq), (4, 0xdead_beef));
    // Input: blob static, 5 000 bytes of 0xa5, in 3 968-byte LEBs: two LEBs.
    let blob = volume(&image, "blob");
    assert_eq!((blob.id, blob.kind), (0, VolumeKind::Static));
    assert_eq!((blob.image_lebs, blob.first_peb), (2, Some(2)));
    assert_eq!(blob.static_bytes, Some(5_000));
    assert_eq!(&bytes[2 * 4096 + 128..][..3_968], &[0xa5u8; 3_968][..]);
}

#[test]
fn spinand_32k_is_read_as_the_builder_wrote_it() {
    let bytes = fixture("spinand-32k.ubi");
    let image = read(&bytes, 32 * 1024).expect("reads");
    assert_eq!(
        image.layout,
        PebLayout {
            peb_size: 32 * 1024,
            vid_header_offset: 2048,
            data_offset: 4096
        }
    );
    assert_eq!((image.pebs, image.image_seq), (3, 7));
    // Inputs: rootfs_a reserves 2 LEBs and holds 9 000 bytes (one LEB);
    // data is autoresize with no LEBs in the image.
    let rootfs_a = volume(&image, "rootfs_a");
    assert_eq!((rootfs_a.reserved_lebs, rootfs_a.image_lebs), (2, 1));
    assert_eq!(rootfs_a.first_peb, Some(2));
    let data = volume(&image, "data");
    assert_eq!((data.id, data.image_lebs), (1, 0));
    assert!(data.autoresize);
    assert_eq!(&bytes[2 * 32_768 + 4096..][..9_000], &[0x6cu8; 9_000][..]);
}

/// Byte offsets of every header in a fixture: each PEB's EC header and its
/// VID header, for the layout of that fixture.
fn header_bytes(peb_size: usize, vid: usize, pebs: usize) -> Vec<usize> {
    let mut at = Vec::new();
    for peb in 0..pebs {
        for start in [0, vid] {
            for byte in 0..HEADER_SIZE {
                at.push(peb * peb_size + start + byte);
            }
        }
    }
    at
}

/// Flipping any one byte of any header must make the reader refuse the
/// image, with the header named. The data bytes are not headers and are not
/// flipped here.
fn every_header_byte_flip_is_refused(name: &str, peb_size: u32, vid: usize, pebs: usize) {
    let bytes = fixture(name);
    let headers = header_bytes(peb_size as usize, vid, pebs);
    assert_eq!(headers.len(), pebs * 2 * HEADER_SIZE);
    for at in headers {
        let mut damaged = bytes.clone();
        damaged[at] ^= 0x01;
        assert!(
            read(&damaged, peb_size).is_err(),
            "{name}: byte {at} flipped and the image still read"
        );
    }
}

#[test]
fn nand_16k_refuses_every_flipped_header_byte() {
    every_header_byte_flip_is_refused("nand-16k.ubi", 16 * 1024, 512, 6);
}

#[test]
fn nor_4k_refuses_every_flipped_header_byte() {
    every_header_byte_flip_is_refused("nor-4k.ubi", 4096, 64, 4);
}

#[test]
fn spinand_32k_refuses_every_flipped_header_byte() {
    every_header_byte_flip_is_refused("spinand-32k.ubi", 32 * 1024, 2048, 3);
}

#[test]
fn a_flipped_static_data_byte_is_refused() {
    // The kernel's LEB in nand-16k.ubi is PEB 4; its data starts at 1024.
    let mut bytes = fixture("nand-16k.ubi");
    bytes[4 * 16_384 + 1024 + 17] ^= 0x01;
    assert!(matches!(
        read(&bytes, 16 * 1024),
        Err(ImageError::StaticDataCrc { peb: 4, .. })
    ));
}
