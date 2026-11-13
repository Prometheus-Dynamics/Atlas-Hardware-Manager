//! The card writer and the image's streams: a two-stream file (the image, then a
//! signature record) writes exactly stream 1 and never the record, and a large
//! single-block image is written in bounded resident memory (Linux only: the peak is
//! read from /proc, reset before the write).
//!
//! The bound: the writer's segment buffers (two of 16 MiB) and the decoder (the
//! dictionary, at most 8 MiB, plus 256 KiB) together stay under 96 MiB. A decoder
//! that held a whole 256 MiB block in memory would exceed it.

mod common;

use std::sync::atomic::AtomicBool;

use atlas_disk::{ResumePolicy, WriteOptions, write_image};
use common::{FileTarget, temp, xz_bytes};

/// A stand-in signature record: the 108 bytes, as the record stream decodes to.
fn record() -> Vec<u8> {
    let mut bytes = b"LMNSSIG1".to_vec();
    bytes.extend_from_slice(&[1, 0, 0, 0]);
    bytes.extend((0..32).map(|i| i as u8)); // key id
    bytes.extend((0..64).map(|i| 0xC0 ^ i as u8)); // signature
    assert_eq!(bytes.len(), 108);
    bytes
}

fn write_to(image: &std::path::Path, disk: &mut FileTarget) {
    write_image(
        image,
        disk,
        &WriteOptions {
            verify: true,
            image_sha256: None,
        },
        &ResumePolicy::default(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .expect("the image is written");
}

#[test]
fn a_two_stream_file_writes_stream_one_and_never_the_record() {
    let raw: Vec<u8> = (0..(3 << 20) + 77u32).map(|i| (i % 249) as u8).collect();
    let mut file = xz_bytes(&raw);
    file.extend_from_slice(&xz_bytes(&record()));
    let image = temp("two-stream.img.xz");
    std::fs::write(&image, &file).unwrap();

    let mut disk = FileTarget::new(4 << 20, 0xAA);
    write_to(&image, &mut disk);
    let written = disk.contents();
    assert_eq!(&written[..raw.len()], &raw[..], "the card holds stream 1 exactly");
    assert!(
        written[raw.len()..].iter().all(|byte| *byte == 0xAA),
        "nothing past the image is written: the record never reaches storage"
    );

    // The same image as a single stream writes the same bytes.
    let single = temp("single.img.xz");
    std::fs::write(&single, xz_bytes(&raw)).unwrap();
    let mut other = FileTarget::new(4 << 20, 0xAA);
    write_to(&single, &mut other);
    assert_eq!(other.contents(), written);
    let _ = std::fs::remove_file(&image);
    let _ = std::fs::remove_file(&single);
}

#[cfg(target_os = "linux")]
fn status_kib(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn a_single_block_image_of_256_mib_is_written_in_bounded_memory() {
    const SIZE: usize = 256 << 20;
    let raw: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let image = temp("big-single-block.img.xz");
    // One block, as `xz -T1` writes it.
    std::fs::write(&image, xz_bytes(&raw)).unwrap();
    drop(raw);

    let mut disk = FileTarget::sparse(SIZE as u64);
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let baseline = status_kib("VmRSS:");
    write_image(
        &image,
        &mut disk,
        &WriteOptions {
            verify: false,
            image_sha256: None,
        },
        &ResumePolicy::default(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .expect("written");
    let grown = status_kib("VmHWM:").saturating_sub(baseline);
    eprintln!(
        "256 MiB single-block image: peak resident grew by {} MiB while writing",
        grown >> 10
    );
    assert!(
        grown < 96 * 1024,
        "the write grew resident memory by {} MiB; the bound is 96 MiB",
        grown >> 10
    );
    let _ = std::fs::remove_file(&image);
}

#[cfg(target_os = "linux")]
#[test]
fn a_two_stream_256_mib_image_is_written_in_bounded_memory() {
    const SIZE: usize = 256 << 20;
    let raw: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let mut file = xz_bytes(&raw);
    file.extend_from_slice(&xz_bytes(&record()));
    drop(raw);
    let image = temp("big-two-stream.img.xz");
    std::fs::write(&image, &file).unwrap();
    drop(file);

    let mut disk = FileTarget::sparse(SIZE as u64);
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let baseline = status_kib("VmRSS:");
    write_image(
        &image,
        &mut disk,
        &WriteOptions {
            verify: false,
            image_sha256: None,
        },
        &ResumePolicy::default(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .expect("written");
    let grown = status_kib("VmHWM:").saturating_sub(baseline);
    eprintln!(
        "256 MiB two-stream image: peak resident grew by {} MiB while writing",
        grown >> 10
    );
    assert!(
        grown < 96 * 1024,
        "the write grew resident memory by {} MiB; the bound is 96 MiB",
        grown >> 10
    );
    let _ = std::fs::remove_file(&image);
}
