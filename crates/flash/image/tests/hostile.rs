//! Untrusted input: truncated, corrupted and mutated images. Every one either
//! reads or is refused with an error; none panics, and none leaves scratch
//! behind.

mod common;

use std::io::Read;

use atlas_image::{Options, open};
use common::*;

/// `n` cut points spread over the whole image, plus the ends.
fn cuts(len: usize, n: usize) -> Vec<usize> {
    let mut points: Vec<usize> = (0..n).map(|i| len * i / n).collect();
    points.extend([
        len.saturating_sub(1),
        len.saturating_sub(12),
        len.saturating_sub(40),
    ]);
    points.sort_unstable();
    points.dedup();
    points
}

/// Reads every part of slot `alpha` the image gives, ignoring the results: the
/// point is that reading a damaged image does not panic. Returns whether the
/// image was read at all.
fn touch_everything(path: &std::path::Path, scratch: &std::path::Path) -> bool {
    let Ok(image) = open(path, scratch, &["alpha"], &Options::default()) else {
        return false;
    };
    let Ok(slot) = image.slot("alpha") else {
        return false;
    };
    let _ = slot.root_hash();
    if let Ok(mut reader) = slot.rootfs() {
        let _ = reader.read_to_end(&mut Vec::new());
    }
    for file in slot.boot_files() {
        if let Ok(mut reader) = slot.boot_file(&file.path) {
            let _ = reader.read_to_end(&mut Vec::new());
        }
    }
    true
}

#[test]
fn a_truncated_xz_is_refused_wherever_it_is_cut() {
    let dir = TestDir::new("trunc-xz");
    let scratch = TestDir::new("trunc-xz-scratch");
    let whole = xz(&image_disk());
    for cut in cuts(whole.len(), 41) {
        if cut == whole.len() {
            continue;
        }
        let path = dir.write("cut.img.xz", &whole[..cut]);
        assert!(
            !touch_everything(&path, scratch.path()),
            "a stream cut at {cut} of {} was read",
            whole.len()
        );
    }
    assert!(
        scratch.entries().is_empty(),
        "left behind: {:?}",
        scratch.entries()
    );
}

#[test]
fn a_truncated_plain_img_is_refused_wherever_it_is_cut() {
    let dir = TestDir::new("trunc-img");
    let scratch = TestDir::new("trunc-img-scratch");
    let whole = image_disk();
    for cut in cuts(whole.len(), 97) {
        if cut == whole.len() {
            continue;
        }
        let path = dir.write("cut.img", &whole[..cut]);
        assert!(
            !touch_everything(&path, scratch.path()),
            "an image cut at {cut} of {} was read",
            whole.len()
        );
    }
    assert!(scratch.entries().is_empty());
}

#[test]
fn corrupt_xz_bytes_are_refused_not_read_as_a_slot() {
    let dir = TestDir::new("corrupt-xz");
    let scratch = TestDir::new("corrupt-xz-scratch");
    let whole = xz(&image_disk());
    let mut positions: Vec<usize> = (0..40).map(|i| whole.len() * i / 40).collect();
    positions.extend([6, 12, 20, whole.len() - 1, whole.len() - 8]);
    for at in positions {
        let mut damaged = whole.clone();
        damaged[at] ^= 0x55;
        let path = dir.write("damaged.img.xz", &damaged);
        assert!(
            !touch_everything(&path, scratch.path()),
            "a flipped byte at {at} of {} was read as a slot",
            whole.len()
        );
    }
    assert!(scratch.entries().is_empty());
}

#[test]
fn a_damaged_partition_table_is_refused() {
    let dir = TestDir::new("gpt");
    let scratch = TestDir::new("gpt-scratch");
    let whole = image_disk();
    // The header's signature, its CRC, its entry array, and a partition name.
    for at in [
        512,
        512 + 20,
        512 + 88,
        1024,
        1024 + 60,
        1024 + 40 * 128 + 33,
    ] {
        let mut damaged = whole.clone();
        damaged[at] ^= 0x01;
        let path = dir.write("gpt.img", &damaged);
        assert!(
            !touch_everything(&path, scratch.path()),
            "table byte {at} changed, still read"
        );
    }
    assert!(scratch.entries().is_empty());
}

/// A deterministic generator, so a failure can be replayed from its seed.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Random damage, a fixed seed and 40 runs by default, so the gate stays quick.
/// `ATLAS_IMAGE_DAMAGE_RUNS=N` runs N (for a longer local check).
#[test]
fn random_damage_never_panics_the_reader() {
    let runs: usize = std::env::var("ATLAS_IMAGE_DAMAGE_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    let dir = TestDir::new("mutate");
    let scratch = TestDir::new("mutate-scratch");
    let whole = image_disk();
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    for _ in 0..runs {
        let mut damaged = whole.clone();
        let count = 2 + (rng.next() % 3) as usize;
        for _ in 0..count {
            let at = (rng.next() as usize) % damaged.len();
            damaged[at] = rng.next() as u8;
        }
        let path = dir.write("mutated.img", &damaged);
        touch_everything(&path, scratch.path());
        assert!(
            scratch.entries().is_empty(),
            "left behind: {:?}",
            scratch.entries()
        );
    }
}
