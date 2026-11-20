//! The files Gaia's `gaia.flash.image` writes (Gaia next 94886210), unsigned and signed: one image stream of four
//! blocks (64 MiB, 64 MiB, 64 MiB, 1 MiB + 7 bytes; block 1 all zeros), CRC64, LZMA2 preset 6 (8 MiB dictionary),
//! sizes in every block header; the signed file is the unsigned stream byte for byte, then one more stream of one
//! block that decodes to the 108-byte `LMNSSIG1` record (board "flash-test", format "disk", Gaia's TEST key, seed
//! [7; 32], not a real key). Made by Gaia's test a_multi_block_image_is_read_back_whole_unsigned_and_signed
//! (crates/nodes/flash-image/tests/flash.rs). Gaia's sha256 values are checked, so a changed fixture cannot pass; the
//! decoded image (193 MiB) is streamed into a hasher, never held in memory. The record's signature is checked where
//! records are verified (atlas-bundle), not here: this crate reads the streams.

use std::io::{Cursor, Write};

use atlas_xzdec::{DICT_CEILING, decode_image, read_record, scan};
use sha2::{Digest, Sha256};

const UNSIGNED: &[u8] = include_bytes!("fixtures/gaia-94886210-4blocks-unsigned.img.xz");
const UNSIGNED_SHA256: &str = "b667d9daee141482d2e58e479f70a427eab014234487b05b48244997596a7ef6";
const SIGNED: &[u8] = include_bytes!("fixtures/gaia-94886210-4blocks-signed.img.xz");
const SIGNED_SHA256: &str = "19e0511907b2b1e668c04daa630f4613cbaf39b4fc9bf12eedfdeab6d1aa11b9";
/// 3 x 64 MiB + 1 MiB + 7.
const IMAGE_BYTES: u64 = 202_375_175;
const IMAGE_SHA256: &str = "1930cb1b01d0e4ba608a743e78cd123ef516d415502bf1a7907bfc1d2aeef4e9";
/// The TEST key's id: the sha256 of its public key (Gaia's README for the fixtures).
const TEST_KEY_ID: &str = "fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A sink that counts and hashes what it is given, and keeps none of it.
struct Hashing {
    hasher: Sha256,
    count: u64,
}

impl Write for Hashing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.count += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decoded(file: &[u8]) -> (u64, String) {
    let mut input = Cursor::new(file);
    let layout = scan(&mut input).expect("scan");
    let mut sink = Hashing {
        hasher: Sha256::new(),
        count: 0,
    };
    decode_image(&mut input, &layout, &mut sink).expect("the image decodes");
    (sink.count, hex(&sink.hasher.finalize()))
}

#[test]
fn the_fixtures_are_the_files_gaia_sent() {
    assert_eq!(hex(&Sha256::digest(UNSIGNED)), UNSIGNED_SHA256);
    assert_eq!(UNSIGNED.len(), 37_964);
    assert_eq!(hex(&Sha256::digest(SIGNED)), SIGNED_SHA256);
    assert_eq!(SIGNED.len(), 38_132);
    assert_eq!(
        &SIGNED[..UNSIGNED.len()],
        UNSIGNED,
        "the signed file's first stream is the unsigned file"
    );
}

#[test]
fn the_unsigned_file_is_one_image_stream_of_four_blocks_and_no_record() {
    let layout = scan(&mut Cursor::new(UNSIGNED)).expect("a first-flash image");
    assert_eq!(layout.image.start, 0);
    assert_eq!(
        layout.image.len,
        UNSIGNED.len() as u64,
        "the whole file is the image stream"
    );
    assert_eq!(layout.record, None);
    assert_eq!(layout.blocks, 4, "64, 64, 64 MiB and 1 MiB + 7");
    assert_eq!(layout.dictionary, DICT_CEILING, "preset 6: 8 MiB");
    assert_eq!(layout.uncompressed, IMAGE_BYTES);
    assert!(!layout.plain);
}

#[test]
fn the_signed_file_is_the_same_image_stream_then_a_record_stream_with_no_padding() {
    let layout = scan(&mut Cursor::new(SIGNED)).expect("a first-flash image");
    assert_eq!(layout.image.start, 0);
    assert_eq!(
        layout.image.len,
        UNSIGNED.len() as u64,
        "stream 1 is the unsigned file"
    );
    let record = layout.record.expect("a record stream");
    assert_eq!(
        record.start,
        UNSIGNED.len() as u64,
        "Gaia writes no padding between the streams"
    );
    assert_eq!(
        record.start + record.len,
        SIGNED.len() as u64,
        "nothing after the record stream"
    );
    assert_eq!(layout.blocks, 4);
    assert_eq!(layout.dictionary, DICT_CEILING);
    assert_eq!(layout.uncompressed, IMAGE_BYTES);
}

#[test]
fn both_files_decode_to_the_stated_image_and_the_record_never_reaches_it() {
    assert_eq!(decoded(UNSIGNED), (IMAGE_BYTES, IMAGE_SHA256.to_string()));
    assert_eq!(
        decoded(SIGNED),
        (IMAGE_BYTES, IMAGE_SHA256.to_string()),
        "the signed file's image is the same bytes: the record is not decoded into it"
    );
}

#[test]
fn the_record_is_108_bytes_of_lmnssig1_naming_the_test_key() {
    let mut input = Cursor::new(SIGNED);
    let layout = scan(&mut input).expect("scan");
    let record = read_record(&mut input, &layout).expect("the record stream decodes");
    assert_eq!(record.len(), 108);
    assert_eq!(&record[..8], b"LMNSSIG1");
    let id = hex(&record);
    assert!(
        id.contains(TEST_KEY_ID),
        "the record names the TEST key's id"
    );
}

#[test]
fn the_unsigned_file_has_no_record_to_read() {
    let mut input = Cursor::new(UNSIGNED);
    let layout = scan(&mut input).expect("scan");
    assert!(read_record(&mut input, &layout).is_err());
}
