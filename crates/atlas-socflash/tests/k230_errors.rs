//! Failure paths of the K230 burner against the C source (burner_uboot.cpp,
//! "U:n"; see k230_vectors.rs for the key).

mod k230_common;

use atlas_socflash::k230::kburn::{Burner, Medium};
use atlas_socflash::{Script, Step, UsbError};
use k230_common::*;

#[tokio::test]
async fn probe_chunk_sizes_follow_the_c_limits() {
    // U:524-533: out chunk 1..=INT_MAX, in chunk 1..=65535; U:518: the
    // reply is exactly two u64.
    for (data, fine) in [
        (words(&[1024, 512]), true),
        (words(&[0x7fff_ffff, 65535]), true),
        (words(&[0, 512]), false),
        (words(&[0x8000_0000, 512]), false),
        (words(&[1024, 0]), false),
        (words(&[1024, 65536]), false),
        (words(&[1024]), false),
        (words(&[1024, 512, 1]), false),
    ] {
        let mut steps = sync(1);
        steps.push(out(0x10, &[2, 0xff]));
        steps.push(rep(0x10, 1, &data));
        let mut script = Script::new(steps);
        let result = Burner::new(&mut script).probe(Medium::Sd).await;
        assert_eq!(result.is_ok(), fine, "{data:02x?}: {result:?}");
    }
}

#[tokio::test]
async fn a_failed_command_names_its_result_and_message() {
    // U:46-54 `kburn_parse_resp` (U:241-293): result 2 is an error; 0xFF
    // carries a message in the data (U:265-268, U:67-84).
    let mut steps = Vec::new();
    ready(&mut steps, 1, 1024, 512);
    steps.push(out(0x20, &words(&[0, 4096])));
    steps.push(rep(0x20, 2, &[]));
    steps.push(out(0x20, &words(&[0, 4096])));
    steps.push(rep(0x20, 0xff, b"erase failed at 0x1000\0junk"));
    steps.push(out(0x20, &words(&[0, 4096])));
    steps.push(rep(0x20, 0, &[]));
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    let plain = burner.erase(0, 4096).await.unwrap_err().to_string();
    assert!(
        plain.contains("0x20") && plain.contains("result 2"),
        "{plain}"
    );
    let message = burner.erase(0, 4096).await.unwrap_err().to_string();
    assert!(message.contains("0xff"), "{message}");
    assert!(message.contains("erase failed at 0x1000"), "{message}");
    assert!(!message.contains("junk"), "{message}");
    let none = burner.erase(0, 4096).await.unwrap_err().to_string();
    assert!(none.contains("result 0"), "{none}");
    assert!(script.finished());
}

#[tokio::test]
async fn stale_replies_are_skipped_up_to_the_c_limit() {
    // U:295-329: a reply with another command, a size over 54, or (for NOP)
    // a result other than OK is discarded; the 9th discard fails
    // (U:14: 8 allowed).
    let wrong_size = Step::BulkIn {
        max: 60,
        reply: {
            let mut reply = pkt(0x8011, 1, &[]);
            reply[4] = 55;
            reply
        },
    };
    let mut steps = vec![out(0x11, &[])];
    steps.push(rep(0x20, 1, &[]));
    steps.push(wrong_size);
    steps.push(rep(0x11, 1, &words(&[1, 2, 3, VALID])));
    let mut script = Script::new(steps);
    let mut burner = Burner::new(&mut script);
    assert_eq!(burner.info().await.unwrap().capacity, 1);

    let mut steps = vec![out(0x11, &[])];
    steps.extend((0..9).map(|_| rep(0x20, 1, &[])));
    let mut script = Script::new(steps);
    let error = Burner::new(&mut script).info().await.unwrap_err();
    assert!(error.to_string().contains("stale"), "{error}");

    // A NOP reply with a failing result is stale too.
    let mut steps = sync(1);
    steps.insert(2, rep(0, 2, &[]));
    steps.extend(probe(2, 512, 512));
    let mut script = Script::new(steps);
    Burner::new(&mut script).probe(Medium::Sd).await.unwrap();
    assert!(script.finished());
}

#[tokio::test]
async fn a_reply_that_is_not_a_whole_packet_is_an_error() {
    // U:171-173 `kburn_read_data` fails unless the full 60 bytes arrive.
    let mut script = Script::new(vec![
        out(0x11, &[]),
        Step::BulkIn {
            max: 60,
            reply: pkt(0x8011, 1, &[])[..40].to_vec(),
        },
    ]);
    let error = Burner::new(&mut script).info().await.unwrap_err();
    assert!(error.to_string().contains("40-byte"), "{error}");
}

#[tokio::test]
async fn a_verify_reply_shorter_than_the_c_layout_is_not_a_match() {
    // U:369-432: exactly digest(32) + verified bytes(8) + elapsed(8) = 48;
    // anything else, or a different byte count, fails.
    let digest = [0xabu8; 32];
    let good = |count: u64, extra: usize| {
        let mut data = digest.to_vec();
        data.extend(words(&[count, 5]));
        data.resize(48 + extra, 0);
        data
    };
    let replies = [
        (digest[..31].to_vec(), false),
        (digest.to_vec(), false),
        (good(512, 0)[..47].to_vec(), false),
        (good(512, 0), true),
        (good(512, 6), false),
        (good(511, 0), false),
    ];
    for (data, fine) in replies {
        let mut steps = Vec::new();
        ready(&mut steps, 2, 512, 512);
        steps.push(out(0x25, &words(&[0, 512])));
        steps.push(rep(0x25, 1, &data));
        let mut script = Script::new(steps);
        let mut burner = start(&mut script).await;
        let result = burner.sha256(0, 512).await;
        assert_eq!(result.is_ok(), fine, "{} bytes: {result:?}", data.len());
        if let Ok(hex) = result {
            assert_eq!(hex, "ab".repeat(32));
        }
    }
}

#[tokio::test]
async fn a_verify_the_burner_refuses_reports_its_message() {
    let mut steps = Vec::new();
    ready(&mut steps, 2, 512, 512);
    steps.push(out(0x25, &words(&[0, 512])));
    steps.push(rep(0x25, 0xff, b"read error"));
    let mut script = Script::new(steps);
    let error = start(&mut script).await.sha256(0, 512).await.unwrap_err();
    assert!(error.to_string().contains("read error"), "{error}");
}

#[tokio::test]
async fn a_disconnect_during_a_data_write_is_reported_as_one() {
    // U:735-748: when a chunk's bulk write fails, the C tool reads the
    // WRITE_LBA reply and reports its message. The disconnect case passes
    // through untouched.
    let mut steps = Vec::new();
    ready(&mut steps, 2, 512, 512);
    steps.push(out(0x21, &words(&[0, 512, 0, 0])));
    steps.push(rep(0x21, 1, &[]));
    steps.push(Step::Disconnect);
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    let error = burner
        .write(0, &mut &[1u8; 512][..], 512, 0, &mut |_, _, _| {}, &|| {
            false
        })
        .await
        .unwrap_err();
    assert_eq!(error, UsbError::Disconnected);
}
