#![cfg(unix)]
//! The card writer on a multi-block image: an xz stream of many independently
//! decodable blocks, as a threaded encoder writes it, is written exactly, with
//! and without a signature record after it. The record never reaches storage.
//! The file is checked to have more than one block first.

use crate::common;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

use atlas_disk::{ResumePolicy, WriteOptions, write_image};
use common::{FileTarget, temp};

/// The record stream of a 108-byte stand-in record, as the signer makes it.
fn record() -> Vec<u8> {
    let mut bytes = b"LMNSSIG1".to_vec();
    bytes.extend_from_slice(&[1, 0, 0, 0]);
    bytes.extend((0..32).map(|i| i as u8));
    bytes.extend((0..64).map(|i| 0xC0 ^ i as u8));
    bytes
}

/// `raw` as one xz stream written by the `xz` program with `options`.
fn xz_with(raw: &[u8], options: &[&str]) -> Vec<u8> {
    use std::io::Write;
    let mut child = Command::new("xz")
        .args(options)
        .args(["--compress", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the xz program is needed to make the test images");
    let mut stdin = child.stdin.take().expect("stdin");
    let input = raw.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().expect("xz");
    writer.join().expect("writer").expect("write to xz");
    assert!(output.status.success(), "xz failed");
    output.stdout
}

fn blocks_in(path: &Path) -> u64 {
    let output = Command::new("xz")
        .args(["--robot", "--list"])
        .arg(path)
        .output()
        .expect("xz");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines()
        .find(|line| line.starts_with("file\t"))
        .and_then(|line| line.split('\t').nth(2).and_then(|b| b.parse().ok()))
        .expect("the block count")
}

fn write_to(image: &Path, disk: &mut FileTarget) {
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
fn a_threaded_multi_block_image_is_written_exactly() {
    let raw: Vec<u8> = (0..(3 << 20) + 77u32).map(|i| (i % 249) as u8).collect();
    let image = temp("multi-block.img.xz");
    std::fs::write(
        &image,
        xz_with(&raw, &["--check=crc64", "-T4", "--block-size=262144"]),
    )
    .unwrap();
    assert!(blocks_in(&image) > 1, "the stream must have several blocks");

    let mut disk = FileTarget::new(4 << 20, 0xAA);
    write_to(&image, &mut disk);
    let written = disk.contents();
    assert_eq!(&written[..raw.len()], &raw[..], "the card holds the image");
    assert!(written[raw.len()..].iter().all(|byte| *byte == 0xAA));
    let _ = std::fs::remove_file(&image);
}

#[test]
fn a_multi_block_image_with_a_record_writes_the_image_and_never_the_record() {
    let raw: Vec<u8> = (0..(3 << 20) + 77u32).map(|i| (i % 249) as u8).collect();
    let mut file = xz_with(&raw, &["--check=crc64", "-T4", "--block-size=262144"]);
    file.extend_from_slice(&xz_with(&record(), &["-0", "--check=crc64"]));
    let image = temp("multi-block-record.img.xz");
    std::fs::write(&image, &file).unwrap();

    let mut disk = FileTarget::new(4 << 20, 0xAA);
    write_to(&image, &mut disk);
    let written = disk.contents();
    assert_eq!(&written[..raw.len()], &raw[..], "the card holds the image");
    assert!(
        written[raw.len()..].iter().all(|byte| *byte == 0xAA),
        "the record never reaches storage"
    );
    let _ = std::fs::remove_file(&image);
}

/// Gaia's sample: one stream of three 64 MiB-block blocks, from the xzdec fixtures
/// (131 KB; the 160 MiB it decodes to is written, never kept in a buffer here).
#[test]
fn gaia_s_sample_is_written_to_a_sparse_card_with_its_length_and_digest() {
    use sha2::{Digest, Sha256};
    let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../xzdec/tests/fixtures/gaia-64mib-3blocks.img.xz");
    let image = temp("gaia-sample.img.xz");
    std::fs::copy(&sample, &image).unwrap();
    let mut disk = FileTarget::sparse(167_772_160);
    write_to(&image, &mut disk);
    let written = disk.contents();
    assert_eq!(written.len(), 167_772_160);
    let digest: String = Sha256::digest(&written)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest,
        "326c678268e0a2ca05ce03d3c5cc2b0737f6137bb484a53017a998cfd507a16f"
    );
    let _ = std::fs::remove_file(&image);
}
