//! Writing keeps memory flat whatever the image's size: decoded data streams
//! through fixed buffers, and the read-back compares segment by segment. A
//! 128 MiB image must not take more than the bound below. It reads /proc, so
//! it runs on Linux only.

#![cfg(target_os = "linux")]

mod common;

use std::sync::atomic::AtomicBool;

use atlas_disk::{ResumePolicy, WriteOptions, write_image};
use common::{FileTarget, temp};

fn peak_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

#[test]
fn a_large_image_is_written_in_bounded_memory() {
    const SIZE: u64 = 128 << 20;
    let image = temp("big.img");
    std::fs::File::create(&image)
        .unwrap()
        .set_len(SIZE)
        .unwrap();
    let mut disk = FileTarget::sparse(SIZE);

    let report = write_image(
        &image,
        &mut disk,
        &WriteOptions {
            verify: true,
            image_sha256: None,
        },
        &ResumePolicy::default(),
        None,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    let peak = peak_kib();
    let _ = std::fs::remove_file(&image);
    drop(disk);
    let report = report.unwrap();
    assert_eq!(report.bytes_written, SIZE);
    assert!(report.verified);
    // Two 16 MiB segment buffers, the decoder and the test harness: well under 96 MiB.
    assert!(peak < 96 * 1024, "peak resident memory {} MiB", peak / 1024);
}
