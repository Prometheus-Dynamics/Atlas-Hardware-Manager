//! A sample written by Gaia's encoder: one xz stream, `xz -6 --check=crc64
//! --block-size=64MiB -T4`, three blocks of 64, 64 and 32 MiB, sizes in the block
//! headers, LZMA2 with an 8 MiB dictionary, no record stream. The fixture is
//! 131 KB; the decoded image (160 MiB) is never held in memory: it is streamed into
//! a hasher. Gaia's sha256 values are checked, so a changed fixture cannot pass.

use std::io::{Cursor, Write};

use atlas_xzdec::{DICT_CEILING, MAX_BLOCKS, decode_image, scan};
use sha2::{Digest, Sha256};

const FIXTURE: &[u8] = include_bytes!("fixtures/gaia-64mib-3blocks.img.xz");
const FIXTURE_SHA256: &str = "87de9f1d5b4f15861242925106a7661459fa614b1a9e0f450ce8ea747d1deff0";
const IMAGE_BYTES: u64 = 167_772_160;
const IMAGE_SHA256: &str = "326c678268e0a2ca05ce03d3c5cc2b0737f6137bb484a53017a998cfd507a16f";
/// Gaia's block size: 64 MiB.
const GAIA_BLOCK: u64 = 64 << 20;

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

#[test]
fn the_fixture_is_the_file_gaia_sent() {
    assert_eq!(hex(&Sha256::digest(FIXTURE)), FIXTURE_SHA256);
    assert_eq!(FIXTURE.len(), 131_020);
}

#[test]
fn scan_reports_one_image_stream_of_three_blocks_with_an_8_mib_dictionary_and_no_record() {
    let mut input = Cursor::new(FIXTURE);
    let layout = scan(&mut input).expect("the sample is a first-flash image");
    assert_eq!(layout.image.start, 0);
    assert_eq!(
        layout.image.len,
        FIXTURE.len() as u64,
        "the whole file is the image stream"
    );
    assert_eq!(layout.record, None, "no record stream");
    assert_eq!(layout.blocks, 3, "64, 64 and 32 MiB");
    assert_eq!(layout.dictionary, DICT_CEILING, "8 MiB");
    assert_eq!(layout.uncompressed, IMAGE_BYTES);
    assert!(!layout.plain);
}

#[test]
fn decoding_streams_exactly_the_stated_bytes_into_the_stated_digest() {
    let mut input = Cursor::new(FIXTURE);
    let layout = scan(&mut input).expect("scan");
    let mut sink = Hashing {
        hasher: Sha256::new(),
        count: 0,
    };
    decode_image(&mut input, &layout, &mut sink).expect("the image decodes");
    assert_eq!(sink.count, IMAGE_BYTES);
    assert_eq!(hex(&sink.hasher.finalize()), IMAGE_SHA256);
}

#[test]
fn gaia_block_count_arithmetic_matches_the_block_limit() {
    // blocks = ceil(size / 64 MiB), for the sample and at the limit.
    let blocks_for = |size: u64| size.div_ceil(GAIA_BLOCK);
    assert_eq!(blocks_for(IMAGE_BYTES), 3);
    assert_eq!(blocks_for(GAIA_BLOCK), 1);
    assert_eq!(blocks_for(GAIA_BLOCK + 1), 2);
    // The limit is reached at 512 GiB: MAX_BLOCKS blocks of 64 MiB.
    assert_eq!(
        MAX_BLOCKS, 8192,
        "a change here must be matched in Gaia's writer"
    );
    assert_eq!(MAX_BLOCKS * GAIA_BLOCK, 512 << 30);
    assert_eq!(blocks_for(512 << 30), MAX_BLOCKS);
    assert_eq!(blocks_for((512 << 30) + 1), MAX_BLOCKS + 1);
}

#[test]
#[ignore = "awaiting Gaia's signed sample (image + signature stream); TODO: add the file and check its record"]
fn a_signed_gaia_sample_has_its_image_then_its_record() {
    // TODO(Gaia): when Gaia's signed sample lands, add it under tests/fixtures and check:
    // scan reports the image stream (3 blocks, 8 MiB dictionary) with a record span after
    // it, decode_image yields the 167,772,160 bytes of the image, read_record gives 108 bytes.
    panic!("no signed sample yet");
}
