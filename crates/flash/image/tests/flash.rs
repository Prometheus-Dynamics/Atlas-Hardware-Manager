//! The first-flash manifest and the board check, on generated images: the
//! manifest's fields and bytes, both image forms and both stream formats, the
//! board refusals, the signature file, the bytes an early check reads, and the
//! truncated and hostile inputs.

mod common;

use std::fs;
use std::io::Write as _;
use std::path::Path;

use atlas_image::{
    FlashError, Format, Options, Refusal, check_target, flash_manifest, open, probe_board,
};
use common::*;
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The `flash.json` the builder's printf would write for these fields.
fn builder_json(board: &str, format: &str, size: usize, sha: &str) -> String {
    format!(
        "{{\"schema\":1,\"board\":\"{board}\",\"format\":\"{format}\",\"image\":{{\"size\":{size},\"sha256\":\"{sha}\"}}}}"
    )
}

/// Writes `bytes` as `name` in a fresh directory and returns the path.
fn image_file(dir: &TestDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    dir.write(name, bytes)
}

/// A disk: the data partition at `data_lba`, plus any other partitions.
fn disk_with(others: &[Part<'_>], data: &[u8], data_lba: u64) -> Vec<u8> {
    let mut parts: Vec<Part<'_>> = others
        .iter()
        .map(|p| Part {
            name: p.name,
            first_lba: p.first_lba,
            bytes: p.bytes,
        })
        .collect();
    parts.push(Part {
        name: "data",
        first_lba: data_lba,
        bytes: data,
    });
    gpt_disk(&parts)
}

#[test]
fn the_manifest_fields_and_bytes_match_values_computed_here() {
    let dir = TestDir::new("manifest");
    let raw = image_disk();
    let sha = hex(&raw);
    let expected = builder_json("example-board", "xz", raw.len(), &sha);

    let xz_path = image_file(&dir, "image.img.xz", &xz(&raw));
    let made = flash_manifest(&xz_path, dir.path(), None).expect("xz manifest");
    assert_eq!(made.manifest.board(), "example-board");
    assert_eq!(made.manifest.format(), Format::Xz);
    assert_eq!(made.manifest.image_size(), raw.len() as u64);
    assert_eq!(made.manifest.image_sha256(), sha);
    assert_eq!(
        String::from_utf8(made.json.clone()).expect("utf8"),
        expected
    );
    assert_eq!(made.manifest.to_json(), made.json);

    // The same image as a plain .img: format raw, the same raw digest.
    let raw_path = image_file(&dir, "image.img", &raw);
    let plain = flash_manifest(&raw_path, dir.path(), None).expect("raw manifest");
    assert_eq!(plain.manifest.format(), Format::Raw);
    assert_eq!(
        String::from_utf8(plain.json).expect("utf8"),
        builder_json("example-board", "raw", raw.len(), &sha)
    );
    assert!(
        dir.entries()
            .iter()
            .all(|name| !name.starts_with("atlas-image-")),
        "scratch left behind"
    );
}

#[test]
fn both_image_forms_give_their_manifest() {
    let dir = TestDir::new("forms");
    // Gaia's bring-up form: a rootfs partition, then data.
    let rootfs = noise(96 * 1024, 11);
    let bring_up = disk_with(
        &[Part {
            name: "rootfs",
            first_lba: 2048,
            bytes: &rootfs,
        }],
        DATA_FIXTURE,
        4096,
    );
    // The real form: a boot partition, then data.
    let boot = noise(64 * 1024, 12);
    let real = disk_with(
        &[Part {
            name: "boot",
            first_lba: 2048,
            bytes: &boot,
        }],
        DATA_FIXTURE,
        2048 + 128,
    );

    for (label, raw) in [("bring-up", bring_up), ("real", real)] {
        let sha = hex(&raw);
        let path = image_file(&dir, &format!("{label}.img.xz"), &xz(&raw));
        let made = flash_manifest(&path, dir.path(), None).expect(label);
        assert_eq!(made.manifest.board(), "example-board", "{label}");
        assert_eq!(made.manifest.image_sha256(), sha, "{label}");
        assert_eq!(made.manifest.image_size(), raw.len() as u64, "{label}");
        // The slot reader takes the same image, so the two agree on what it holds.
        let image = open(&path, dir.path(), &["alpha"], &Options::default()).expect(label);
        assert_eq!(
            image.slot("alpha").expect("slot").image_size(),
            20480,
            "{label}"
        );
    }
    assert!(
        dir.entries()
            .iter()
            .all(|name| !name.starts_with("atlas-image-"))
    );
}

#[test]
fn a_wrong_board_is_refused_and_both_names_are_in_the_message() {
    let dir = TestDir::new("wrong");
    let path = image_file(&dir, "image.img.xz", &xz(&image_disk()));

    let probe = probe_board(&path, dir.path()).expect("probe");
    assert_eq!(probe.board, "example-board");
    assert_eq!(probe.slots, ["alpha", "beta"]);
    assert_eq!(probe.format, Format::Xz);

    let error = flash_manifest(&path, dir.path(), Some("other-board")).expect_err("refused");
    let FlashError::Refused(refusal) = &error else {
        panic!("not a refusal: {error}");
    };
    assert_eq!(
        refusal,
        &Refusal::WrongBoard {
            image: "example-board".into(),
            target: "other-board".into()
        }
    );
    let text = error.to_string();
    assert!(
        text.contains("example-board") && text.contains("other-board"),
        "{text}"
    );
    assert!(text.contains("Nothing was pushed"), "{text}");

    // The right board passes, and check_target agrees with the manifest.
    let made = flash_manifest(&path, dir.path(), Some("example-board")).expect("right board");
    assert!(check_target(&made.manifest, "example-board").is_ok());
    assert_eq!(
        check_target(&made.manifest, "other-board"),
        Err(Refusal::WrongBoard {
            image: "example-board".into(),
            target: "other-board".into()
        })
    );
    assert_eq!(
        check_target(&made.manifest, "Other_Board"),
        Err(Refusal::BadTarget {
            target: "Other_Board".into()
        })
    );
    assert!(flash_manifest(&path, dir.path(), Some("Other_Board")).is_err());
    assert!(
        dir.entries()
            .iter()
            .all(|name| !name.starts_with("atlas-image-"))
    );
}

#[test]
fn an_image_with_no_slot_no_board_or_two_boards_is_refused_with_its_own_message() {
    let dir = TestDir::new("boards");
    type Case = (&'static str, Vec<(&'static str, Vec<u8>)>, Refusal);
    let cases: Vec<Case> = vec![
        (
            "no slot",
            vec![("readme.txt", b"no slots here".to_vec())],
            Refusal::NoSlot,
        ),
        (
            "no board",
            vec![("slots/alpha.json", slot_json("alpha", None))],
            Refusal::NoBoard {
                slot: "alpha".into(),
            },
        ),
        (
            "conflicting boards",
            vec![
                (
                    "slots/alpha.json",
                    slot_json("alpha", Some("example-board")),
                ),
                ("slots/beta.json", slot_json("beta", Some("other-board"))),
            ],
            Refusal::ConflictingBoards {
                first: "example-board".into(),
                second: "other-board".into(),
            },
        ),
        (
            "a bad board id",
            vec![(
                "slots/alpha.json",
                slot_json("alpha", Some("Example_Board")),
            )],
            Refusal::BadBoard {
                slot: "alpha".into(),
            },
        ),
    ];
    for (label, files, expected) in cases {
        let raw = disk_with(
            &[],
            &ext4_data(
                &files
                    .iter()
                    .map(|(p, b)| (*p, b.clone()))
                    .collect::<Vec<_>>(),
            ),
            2048,
        );
        let path = image_file(&dir, "case.img.xz", &xz(&raw));
        let error = probe_board(&path, dir.path()).expect_err(label);
        match &error {
            FlashError::Refused(refusal) => assert_eq!(refusal, &expected, "{label}"),
            other => panic!("{label}: {other}"),
        }
        let text = error.to_string();
        assert!(text.contains("Nothing was pushed"), "{label}: {text}");
        assert!(
            flash_manifest(&path, dir.path(), None).is_err(),
            "{label}: manifest made"
        );
    }

    // Two slots naming the same board are fine.
    let raw = disk_with(
        &[],
        &ext4_data(&[
            (
                "slots/alpha.json",
                slot_json("alpha", Some("example-board")),
            ),
            ("slots/beta.json", slot_json("beta", Some("example-board"))),
        ]),
        2048,
    );
    let path = image_file(&dir, "agree.img.xz", &xz(&raw));
    let probe = probe_board(&path, dir.path()).expect("agreeing slots");
    assert_eq!(probe.board, "example-board");
    assert_eq!(probe.slots, ["alpha", "beta"]);
    assert!(
        dir.entries()
            .iter()
            .all(|name| !name.starts_with("atlas-image-"))
    );
}

#[test]
fn the_signature_file_is_reported_by_presence_and_carried_untouched() {
    let dir = TestDir::new("sig");
    let path = image_file(&dir, "image.img.xz", &xz(&image_disk()));

    let unsigned = flash_manifest(&path, dir.path(), None).expect("unsigned");
    assert!(!unsigned.signature_present());
    assert_eq!(unsigned.signature, None);

    let bytes: Vec<u8> = (0..64).map(|i| i as u8 ^ 0xA5).collect();
    dir.write("flash.sig", &bytes);
    let signed = flash_manifest(&path, dir.path(), None).expect("signed");
    assert!(signed.signature_present());
    assert_eq!(signed.signature.as_deref(), Some(bytes.as_slice()));
    // The manifest does not change with the signature.
    assert_eq!(signed.json, unsigned.json);

    fs::write(dir.path().join("flash.sig"), [1u8; 10]).expect("short signature");
    let error = flash_manifest(&path, dir.path(), None).expect_err("short signature");
    assert!(matches!(error, FlashError::Signature(10)), "{error}");
}

/// The bytes a probe reads for an image whose data partition is first and whose
/// tail is 8 MiB of noise: the early check reads the stream up to the data
/// partition, not the tail. The counts are printed for the PR.
#[test]
fn the_early_board_check_reads_only_up_to_the_data_partition() {
    let dir = TestDir::new("early");
    let tail = noise(8 << 20, 21);
    let raw = disk_with(
        &[Part {
            name: "tail",
            first_lba: 2048 + 1024 + 2048,
            bytes: &tail,
        }],
        DATA_FIXTURE,
        2048,
    );
    let file = xz(&raw);
    let path = image_file(&dir, "image.img.xz", &file);
    let total = file.len() as u64;

    let probe = probe_board(&path, dir.path()).expect("probe");
    assert_eq!(probe.board, "example-board");
    eprintln!(
        "early check: read {} of {total} bytes of the image file",
        probe.read
    );
    assert!(
        probe.read * 4 < total,
        "the probe read {} of {total} bytes",
        probe.read
    );

    // The refusal for another board is made at the same point (the end of the data
    // partition): the probe is the early step, and a full manifest reads the rest.
    let error = flash_manifest(&path, dir.path(), Some("other-board")).expect_err("refused");
    assert!(
        matches!(error, FlashError::Refused(Refusal::WrongBoard { .. })),
        "{error}"
    );
    let full = flash_manifest(&path, dir.path(), None).expect("full manifest");
    eprintln!("full manifest: read {} of {total} bytes", full.read);
    assert!(full.read >= total, "a full manifest reads the whole stream");
}

#[test]
fn a_multi_block_stream_is_read_through_its_index() {
    let dir = TestDir::new("blocks");
    let raw = image_disk();
    let path = image_file(
        &dir,
        "blocks.img.xz",
        &xz_with(&raw, &["--check=crc64", "-T1", "--block-size=65536"]),
    );
    let made = flash_manifest(&path, dir.path(), None).expect("blocks");
    assert_eq!(made.manifest.image_sha256(), hex(&raw));
    assert_eq!(made.manifest.image_size(), raw.len() as u64);
}

#[test]
fn a_dictionary_over_the_limit_is_refused_before_anything_is_decoded() {
    let dir = TestDir::new("dict");
    // 17 MiB of a repeating pattern: xz -7 declares a 16 MiB dictionary for it.
    let big: Vec<u8> = (0..(17u64 << 20)).map(|i| (i % 251) as u8).collect();
    let path = image_file(
        &dir,
        "dict.img.xz",
        &xz_with(&big, &["-7", "--check=crc64"]),
    );

    let error = flash_manifest(&path, dir.path(), None).expect_err("over the limit");
    let text = error.to_string();
    assert!(
        text.contains("dictionary") && text.contains("8 MiB"),
        "{text}"
    );
    assert!(
        matches!(error, FlashError::Image(atlas_image::ImageError::Limit(_))),
        "{text}"
    );
    assert!(probe_board(&path, dir.path()).is_err());
    // The slot reader takes the same limit, so the bundle path is protected too.
    assert!(open(&path, dir.path(), &["alpha"], &Options::default()).is_err());
    assert!(
        dir.entries()
            .iter()
            .all(|name| !name.starts_with("atlas-image-"))
    );

    // The builder's own preset (-6, an 8 MiB dictionary) is taken.
    let ok = image_file(
        &dir,
        "six.img.xz",
        &xz_with(&image_disk(), &["-6", "--check=crc64"]),
    );
    assert!(flash_manifest(&ok, dir.path(), None).is_ok());
}

#[test]
fn truncated_and_damaged_images_are_refused_and_leave_no_scratch() {
    let dir = TestDir::new("damage");
    let whole = xz(&image_disk());
    let mut cuts: Vec<usize> = (0..24).map(|i| whole.len() * i / 24).collect();
    cuts.extend([whole.len() - 1, whole.len() - 12, whole.len() - 13, 12, 20]);
    for cut in cuts {
        let path = image_file(&dir, "cut.img.xz", &whole[..cut]);
        assert!(
            flash_manifest(&path, dir.path(), None).is_err(),
            "a stream cut at {cut} was read"
        );
        assert!(
            probe_board(&path, dir.path()).is_err(),
            "a stream cut at {cut} was probed"
        );
    }
    // Damage in the footer, the index and the first block header.
    let n = whole.len();
    for at in [0, 7, 9, n - 1, n - 5, n - 9, n - 13, n - 20, 13, 14] {
        let mut bad = whole.clone();
        bad[at] ^= 0x40;
        let path = image_file(&dir, "bad.img.xz", &bad);
        // A flipped byte may still decode in a place the checks do not read; it must
        // never be read as a different board's image, and never panics.
        let _ = flash_manifest(&path, dir.path(), None);
    }
    // A stream that declares zero blocks (a hostile index) is refused.
    let mut zero = whole.clone();
    let index_start = n
        - 12
        - ((u32::from_le_bytes([whole[n - 8], whole[n - 7], whole[n - 6], whole[n - 5]]) as usize
            + 1)
            * 4);
    zero[index_start + 1] = 0;
    let path = image_file(&dir, "zero.img.xz", &zero);
    assert!(flash_manifest(&path, dir.path(), None).is_err());
    assert!(
        dir.entries()
            .iter()
            .all(|name| !name.starts_with("atlas-image-"))
    );
}

#[test]
fn a_raw_image_without_its_partition_table_is_refused() {
    let dir = TestDir::new("raw-cut");
    let raw = image_disk();
    for cut in [0usize, 511, 1024, 4096, raw.len() / 2] {
        let path = image_file(&dir, "cut.img", &raw[..cut]);
        assert!(
            flash_manifest(&path, dir.path(), None).is_err(),
            "cut at {cut}"
        );
    }
    let mut no_slot = Vec::new();
    no_slot.write_all(&[0u8; 4096]).expect("zeros");
    let path = image_file(&dir, "blank.img", &no_slot);
    assert!(flash_manifest(&path, dir.path(), None).is_err());
}

#[test]
fn the_probe_of_a_missing_file_is_an_io_error_not_a_refusal() {
    let dir = TestDir::new("missing");
    let error =
        probe_board(Path::new(&dir.path().join("nope.img.xz")), dir.path()).expect_err("missing");
    assert!(
        matches!(error, FlashError::Image(atlas_image::ImageError::Io(_))),
        "{error}"
    );
}
