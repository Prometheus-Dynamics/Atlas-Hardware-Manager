//! The peak memory of a first-flash pass over a large image compressed as one xz
//! block (the builder's `xz -T1` form). Linux only: the peak is the process's
//! resident high-water mark, reset before the pass, because liblzma allocates
//! through C and a Rust allocator counter would not see it.
//!
//! It is one test in its own binary, so no other test shares the measurement.

#![cfg(target_os = "linux")]

mod common;

use atlas_image::flash_manifest;
use common::*;
use sha2::{Digest, Sha256};

fn status_kib(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .expect("status")
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .expect("status field")
}

/// Resets the peak to the current resident size (Linux 4.0 and later).
fn reset_peak() {
    std::fs::write("/proc/self/clear_refs", "5").expect("clear_refs");
}

#[test]
fn a_large_single_block_image_is_read_in_bounded_memory() {
    const SIZE: usize = 64 << 20;
    let dir = TestDir::new("memory");
    // A repeating pattern compresses fast; the stream is still one block.
    let body: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let raw = gpt_disk(&[
        Part {
            name: "data",
            first_lba: 2048,
            bytes: DATA_FIXTURE,
        },
        Part {
            name: "body",
            first_lba: 4096,
            bytes: &body,
        },
    ]);
    drop(body);
    let sha: String = Sha256::digest(&raw)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let path = dir.write("big.img.xz", &xz_with(&raw, &["--check=crc64", "-T1"]));
    let total = raw.len();
    drop(raw);

    reset_peak();
    let baseline = status_kib("VmRSS:");
    let made = flash_manifest(&path, dir.path(), None);
    let peak = status_kib("VmHWM:");
    let grown = peak.saturating_sub(baseline);
    eprintln!(
        "single-block image of {} MiB: peak resident grew by {} MiB during the pass",
        SIZE >> 20,
        grown >> 10
    );
    // A whole-block decoder holds the image (64 MiB) at once; the streamed decoder
    // holds its dictionary (at most 8 MiB) and its buffers.
    assert!(
        grown < 32 * 1024,
        "the pass grew resident memory by {} MiB",
        grown >> 10
    );

    let made = made.expect("manifest");
    assert_eq!(made.manifest.image_size(), total as u64);
    assert_eq!(made.manifest.image_sha256(), sha);
}
