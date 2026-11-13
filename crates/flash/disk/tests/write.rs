//! Whole-image writes: the checks before the first byte, the bytes that land,
//! and the cancel. All on regular files.

#![cfg(unix)]

mod common;

use std::sync::atomic::AtomicBool;

use atlas_disk::{DiskError, ResumePolicy, WriteOptions, WriteProgress, write_image};
use common::{FileTarget, sha256_hex, temp};

fn small_segments() -> ResumePolicy {
    ResumePolicy {
        segment_bytes: 1 << 20,
        ..ResumePolicy::default()
    }
}

fn verified(image_sha256: Option<String>) -> WriteOptions {
    WriteOptions {
        verify: true,
        image_sha256,
    }
}

#[test]
fn a_compressed_image_is_written_and_verified() {
    let original: Vec<u8> = (0..(2 << 20) + 123u32).map(|i| (i % 253) as u8).collect();
    let compressed = common::xz_bytes(&original);
    let image = temp("image.img.xz");
    std::fs::write(&image, &compressed).unwrap();
    let mut disk = FileTarget::new(4 << 20, 0xAA);
    let mut phases = Vec::new();

    let report = write_image(
        &image,
        &mut disk,
        &verified(Some(sha256_hex(&compressed))),
        &small_segments(),
        None,
        &mut |progress| phases.push(progress),
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(report.bytes_written, original.len() as u64);
    assert_eq!(report.sha256, sha256_hex(&original));
    assert!(report.verified);
    assert_eq!(&disk.contents()[..original.len()], &original[..]);
    assert!(phases.contains(&WriteProgress::Preparing));
    assert!(
        phases
            .iter()
            .any(|phase| matches!(phase, WriteProgress::Writing { .. }))
    );
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_raw_image_written_without_verify_is_not_called_verified() {
    let original = vec![0x5Au8; 3 << 20];
    let image = temp("raw.img");
    std::fs::write(&image, &original).unwrap();
    let mut disk = FileTarget::new(4 << 20, 0);

    let report = write_image(
        &image,
        &mut disk,
        &WriteOptions::default(),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .unwrap();

    assert!(!report.verified);
    assert_eq!(report.bytes_written, original.len() as u64);
    assert_eq!(&disk.contents()[..original.len()], &original[..]);
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_wrong_image_hash_stops_before_touching_the_disk() {
    let image = temp("hash.img");
    std::fs::write(&image, b"image").unwrap();
    let mut disk = FileTarget::new(1 << 20, 0xAA);

    let result = write_image(
        &image,
        &mut disk,
        &verified(Some("00".repeat(32))),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(DiskError::ImageHashMismatch { .. })));
    assert!(disk.contents().iter().all(|byte| *byte == 0xAA));
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_raw_image_larger_than_the_disk_is_refused_before_any_write() {
    let image = temp("big.img");
    std::fs::write(&image, vec![1u8; 2 << 20]).unwrap();
    let mut disk = FileTarget::new(1 << 20, 0xAA);

    let result = write_image(
        &image,
        &mut disk,
        &verified(None),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(DiskError::TooLarge { .. })));
    assert!(disk.contents().iter().all(|byte| *byte == 0xAA));
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_compressed_image_that_decodes_too_large_is_refused_before_any_write() {
    let original = vec![7u8; 2 << 20];
    let compressed = common::xz_bytes(&original);
    let image = temp("big.img.xz");
    std::fs::write(&image, &compressed).unwrap();
    let mut disk = FileTarget::new(1 << 20, 0xAA);

    let result = write_image(
        &image,
        &mut disk,
        &verified(None),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(DiskError::TooLarge { .. })));
    assert!(disk.contents().iter().all(|byte| *byte == 0xAA));
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_zip_is_refused_with_advice() {
    let image = temp("archive.img");
    std::fs::write(&image, b"PK\x03\x04 not an image").unwrap();
    let mut disk = FileTarget::new(1 << 20, 0xAA);

    let result = write_image(
        &image,
        &mut disk,
        &verified(None),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(DiskError::Image(message)) if message.contains("zip")));
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_broken_xz_stream_is_an_image_error_not_a_silent_raw_write() {
    let mut broken = vec![0xFD, b'7', b'z', b'X', b'Z', 0x00];
    broken.extend_from_slice(b"this is not a valid stream at all");
    let image = temp("broken.img.xz");
    std::fs::write(&image, &broken).unwrap();
    let mut disk = FileTarget::new(1 << 20, 0xAA);

    let result = write_image(
        &image,
        &mut disk,
        &verified(None),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(DiskError::Image(_))), "{result:?}");
    let _ = std::fs::remove_file(image);
}

#[test]
fn cancelling_before_the_first_segment_writes_nothing() {
    let image = temp("cancel.img");
    std::fs::write(&image, vec![1u8; 8 << 20]).unwrap();
    let mut disk = FileTarget::new(16 << 20, 0xAA);

    let result = write_image(
        &image,
        &mut disk,
        &verified(None),
        &small_segments(),
        None,
        &mut |_| {},
        &AtomicBool::new(true),
    );

    assert!(matches!(result, Err(DiskError::Cancelled)));
    assert!(disk.contents().iter().all(|byte| *byte == 0xAA));
    let _ = std::fs::remove_file(image);
}
