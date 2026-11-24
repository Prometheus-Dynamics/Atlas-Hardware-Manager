//! Multi-block xz streams through every reader of a first-flash image: the
//! manifest, the probe and the slot reader, with and without a signature record,
//! from a single-threaded and from a threaded encoder, and the block-count bound.
//!
//! The image is one xz stream of many independently decodable blocks, as Gaia's
//! multi-threaded writer makes it (LZMA2, CRC-64, an 8 MiB dictionary). Each
//! test first checks that the file really has more than one block, so a test
//! cannot pass on a single-block stream by accident.

use crate::common;
use std::path::Path;
use std::process::Command;

use std::io::Read;

use atlas_image::{Options, SignatureState, flash_manifest, open, probe_board};
use common::*;
use sha2::{Digest, Sha256};

/// The signature record of the W1 test vector (public test data, draft 2).
const VECTOR_RECORD: &str = "4c4d4e535349473101000000fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889bd894b37df1a05915c0c6baa7606d57c8cc2a6aa10a01451a3e4f67b8edfd517df6794fbcdeeaadfd7f46e099cea00be896e99445a8942f5e7592ec31c1b6c09";

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_all(mut reader: Box<dyn Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    bytes
}

/// The record stream of `record`, as the signer makes it: `xz -0 --check=crc64`.
fn record_stream(record: &[u8]) -> Vec<u8> {
    xz_with(record, &["-0", "--check=crc64"])
}

/// The number of blocks in an xz file, as `xz --robot --list` counts them.
fn blocks_in(path: &Path) -> u64 {
    let output = Command::new("xz")
        .args(["--robot", "--list"])
        .arg(path)
        .output()
        .expect("the xz program is needed to count blocks");
    assert!(output.status.success(), "xz --list failed");
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|line| line.starts_with("file\t"))
        .expect("a file line");
    line.split('\t')
        .nth(2)
        .and_then(|blocks| blocks.parse().ok())
        .expect("the block count")
}

#[test]
fn a_multi_block_stream_reads_through_the_manifest_the_probe_and_the_slot_reader() {
    let dir = TestDir::new("multi-block");
    let scratch = TestDir::new("multi-block-scratch");
    let raw = image_disk();
    let path = dir.write(
        "blocks.img.xz",
        &xz_with(&raw, &["--check=crc64", "-T1", "--block-size=65536"]),
    );
    assert!(blocks_in(&path) > 1, "the stream must have several blocks");

    let made = flash_manifest(&path, scratch.path(), None).expect("manifest");
    assert_eq!(made.manifest.image_sha256(), hex(&Sha256::digest(&raw)));
    assert_eq!(made.manifest.image_size(), raw.len() as u64);
    assert_eq!(made.signature, SignatureState::Unsigned);

    let probed = probe_board(&path, scratch.path()).expect("probe");
    assert_eq!(probed.board, "example-board");

    let image = open(&path, scratch.path(), &["alpha"], &Options::default()).expect("open");
    let alpha = image.slot("alpha").expect("alpha");
    assert_eq!(alpha.root_hash(), alpha_root());
    assert_eq!(read_all(alpha.rootfs().expect("rootfs")), alpha_file());
}

#[test]
fn a_threaded_multi_block_stream_with_a_signature_record_reads_its_image_and_its_record() {
    let dir = TestDir::new("threaded-record");
    let scratch = TestDir::new("threaded-record-scratch");
    let raw = image_disk();
    let single = dir.write("single.img.xz", &xz(&raw));
    let mut both = xz_with(&raw, &["--check=crc64", "-T4", "--block-size=65536"]);
    both.extend_from_slice(&record_stream(&unhex(VECTOR_RECORD)));
    let path = dir.write("threaded.img.xz", &both);
    let image_only = dir.write(
        "threaded-image.img.xz",
        &xz_with(&raw, &["--check=crc64", "-T4", "--block-size=65536"]),
    );
    assert!(
        blocks_in(&image_only) > 1,
        "the image stream must have several blocks"
    );

    let one = flash_manifest(&single, scratch.path(), None).expect("single");
    let two = flash_manifest(&path, scratch.path(), None).expect("threaded with a record");
    assert_eq!(
        one.manifest, two.manifest,
        "the record is not part of the image"
    );
    assert!(matches!(two.signature, SignatureState::Signed { .. }));

    let probed = probe_board(&path, scratch.path()).expect("probe");
    assert_eq!(probed.board, "example-board");
    let image = open(&path, scratch.path(), &["beta"], &Options::default()).expect("open");
    let beta = image.slot("beta").expect("beta");
    assert_eq!(read_all(beta.rootfs().expect("rootfs")), beta_file());
}

#[test]
fn a_stream_with_more_blocks_than_the_reader_takes_is_refused_by_count_before_decoding() {
    // 8193 blocks of 4 KiB: one more than the reader's 8192. The refusal is by the
    // index, so the content does not matter.
    let dir = TestDir::new("too-many-blocks");
    let scratch = TestDir::new("too-many-blocks-scratch");
    let raw = periodic(8193 * 4096);
    let path = dir.write(
        "many.img.xz",
        &xz_with(&raw, &["-0", "--check=crc64", "-T1", "--block-size=4096"]),
    );
    assert!(blocks_in(&path) > 8192);
    let error = flash_manifest(&path, scratch.path(), None).expect_err("refused");
    assert!(error.to_string().contains("8193 blocks"), "{error}");
}
