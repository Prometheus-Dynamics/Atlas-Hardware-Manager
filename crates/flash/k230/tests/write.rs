//! The burner's write path against the simulated device: a write in chunks
//! with a short last block, its SHA-256 verify, the reboot, and the failures
//! (write protection, a refused start, a reset or stall mid-write, cancel).

mod common;

use atlas_k230::burner::{Burner, Medium};
use atlas_k230::hex;
use atlas_k230::packet::{REBOOT_MARK, cmd, words};
use atlas_usb::sim::Answer;
use atlas_usb::{Transfer, UsbError};
use common::*;
use sha2::{Digest, Sha256};

const VALID: u64 = 1 << 47;

/// The loader's answers to a sync, a probe (1 KiB out, 512 in) and an info of
/// a 1 GiB medium with 512-byte blocks and 4 KiB erase blocks.
fn medium(version: u32, flags: u64) -> Vec<Answer> {
    let mut answers = sync(version);
    answers.extend(exchange(cmd::PROBE, 1, &words(&[1024, 512])));
    answers.extend(exchange(
        cmd::GET_INFO,
        1,
        &words(&[1 << 30, 512, 4096, flags]),
    ));
    answers
}

fn image(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 13 % 256) as u8).collect()
}

#[tokio::test]
async fn a_write_goes_in_chunks_with_a_padded_last_block_and_verifies() {
    let data = image(1300);
    let bus = bus();
    let mut script = medium(2, VALID);
    // The start: 1300 bytes round up to 1536 (three 512-byte blocks).
    script.extend(exchange(cmd::WRITE, 1, &[]));
    script.push(Answer::Ok); // 1024 bytes
    script.push(Answer::Ok); // 512 bytes, the last block
    script.push(Answer::Data(reply(cmd::WRITE, 1, &[])));
    script.extend(exchange(cmd::NOP, 1, &[]));
    // The verify: digest, the 1300 bytes it covered, 5 ms.
    let mut digest_reply = Sha256::digest(&data).to_vec();
    digest_reply.extend(words(&[1300, 5]));
    script.extend(exchange(cmd::VERIFY, 1, &digest_reply));
    bus.script(LOCATION, script);

    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let mut source: &[u8] = &data;
    let mut progress_calls = Vec::new();
    let mut progress = |done: u64, total: u64| progress_calls.push((done, total));
    let digest = burner
        .write(0x1_0000, &mut source, 1300, 0, &mut progress, &|| false)
        .await
        .unwrap();
    assert_eq!(digest, hex(&Sha256::digest(&data)));
    assert_eq!(progress_calls.last(), Some(&(1300, 1300)));
    assert_eq!(burner.sha256(0x1_0000, 1300).await.unwrap(), digest);

    // The first chunk is 1024 bytes of image; the last is the rest and zeros.
    let bulk: Vec<Vec<u8>> = bus
        .records(LOCATION)
        .into_iter()
        .filter_map(|record| match record.transfer {
            Transfer::BulkOut(bytes) if bytes.len() > 60 => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(bulk.len(), 2);
    assert_eq!(bulk[0], data[..1024].to_vec());
    assert_eq!(bulk[1][..276], data[1024..1300]);
    assert!(bulk[1][276..].iter().all(|&byte| byte == 0));
    // The start packet carries offset, padded size, region and flags.
    let start = bus
        .records(LOCATION)
        .into_iter()
        .find_map(|record| match record.transfer {
            Transfer::BulkOut(bytes) if bytes[0] == cmd::WRITE as u8 => Some(bytes),
            _ => None,
        })
        .unwrap();
    assert_eq!(&start[6..14], &0x1_0000u64.to_le_bytes());
    assert_eq!(&start[14..22], &1536u64.to_le_bytes());
}

#[tokio::test]
async fn a_verify_that_covers_another_count_is_not_a_match() {
    let bus = bus();
    let mut script = medium(2, VALID);
    let mut digest_reply = vec![0xabu8; 32];
    digest_reply.extend(words(&[1000, 5]));
    script.extend(exchange(cmd::VERIFY, 1, &digest_reply));
    bus.script(LOCATION, script);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let error = burner.sha256(0, 1300).await.unwrap_err();
    assert!(error.to_string().contains("1000"), "{error}");
}

#[tokio::test]
async fn a_loader_without_verify_refuses_the_check_without_a_transfer() {
    let bus = bus();
    bus.script(LOCATION, medium(1, VALID));
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let before = bus.records(LOCATION).len();
    let error = burner.sha256(0, 16).await.unwrap_err();
    assert!(error.to_string().contains("verification"), "{error}");
    assert_eq!(bus.records(LOCATION).len(), before);
}

#[tokio::test]
async fn the_reboot_sends_the_mark_and_the_reset_after_it_is_success() {
    // The mark goes out as a plain packet first.
    let bus = bus();
    bus.script(LOCATION, vec![Answer::Ok]);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    Burner::new(&mut *io).reboot().await.unwrap();
    let sent = &bus.records(LOCATION)[0].transfer;
    let Transfer::BulkOut(bytes) = sent else {
        panic!("{sent:?}");
    };
    assert_eq!(bytes[0], cmd::REBOOT as u8);
    assert_eq!(&bytes[6..14], &REBOOT_MARK.to_le_bytes());

    // The board resets on it: the disconnect is the reboot, not an error.
    let bus = bus_with(LOCATION);
    bus.script(LOCATION, vec![Answer::Disconnect]);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    Burner::new(&mut *io).reboot().await.unwrap();
    assert!(bus.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_write_is_refused_before_probe_and_on_a_protected_medium() {
    let bus = bus();
    bus.script(LOCATION, medium(2, VALID | (1 << 32)));
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    let mut source: &[u8] = &[1, 2, 3];
    let early = burner
        .write(0, &mut source, 3, 0, &mut |_, _| {}, &|| false)
        .await
        .unwrap_err();
    assert!(early.to_string().contains("before probe"), "{early}");

    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let erase = burner.erase(0, 4096).await.unwrap_err();
    assert!(erase.to_string().contains("write-protected"), "{erase}");
    let mut source: &[u8] = &[1, 2, 3];
    let protected = burner
        .write(0, &mut source, 3, 0, &mut |_, _| {}, &|| false)
        .await
        .unwrap_err();
    assert!(
        protected.to_string().contains("write-protected"),
        "{protected}"
    );
}

#[tokio::test]
async fn a_write_must_start_on_an_erase_block_and_stay_on_the_medium() {
    let bus = bus();
    bus.script(LOCATION, medium(2, VALID));
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let before = bus.records(LOCATION).len();
    let mut source: &[u8] = &[1u8; 16];
    let unaligned = burner
        .write(0x100, &mut source, 16, 0, &mut |_, _| {}, &|| false)
        .await
        .unwrap_err();
    assert!(unaligned.to_string().contains("aligned"), "{unaligned}");
    let mut source: &[u8] = &[1u8; 16];
    let past = burner
        .write(1 << 30, &mut source, 16, 0, &mut |_, _| {}, &|| false)
        .await
        .unwrap_err();
    assert!(past.to_string().contains("past the medium"), "{past}");
    assert_eq!(
        bus.records(LOCATION).len(),
        before,
        "refused before any write was sent"
    );
}

#[tokio::test]
async fn a_reset_mid_write_is_reported_and_no_reply_is_read() {
    let bus = bus();
    let mut script = medium(2, VALID);
    script.extend(exchange(cmd::WRITE, 1, &[]));
    script.push(Answer::Disconnect);
    bus.script(LOCATION, script);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let mut source: &[u8] = &image(1024);
    let data = image(1024);
    let mut source2: &[u8] = &data;
    let _ = &mut source;
    let error = burner
        .write(0x1_0000, &mut source2, 1024, 0, &mut |_, _| {}, &|| false)
        .await
        .unwrap_err();
    assert_eq!(error, UsbError::Disconnected);
}

#[tokio::test]
async fn a_stalled_chunk_reports_the_burner_reason_when_it_gives_one() {
    let bus = bus();
    let mut script = medium(2, VALID);
    script.extend(exchange(cmd::WRITE, 1, &[]));
    script.push(Answer::Stall);
    script.push(Answer::Data(reply(cmd::WRITE, 2, &[])));
    bus.script(LOCATION, script);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let data = image(1024);
    let mut source: &[u8] = &data;
    let error = burner
        .write(0x1_0000, &mut source, 1024, 0, &mut |_, _| {}, &|| false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("result 2"), "{error}");
}

#[tokio::test]
async fn a_cancel_stops_the_write_before_the_next_chunk() {
    let bus = bus();
    let mut script = medium(2, VALID);
    script.extend(exchange(cmd::WRITE, 1, &[]));
    script.push(Answer::Ok);
    bus.script(LOCATION, script);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let mut burner = Burner::new(&mut *io);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    let data = image(2048);
    let mut source: &[u8] = &data;
    let mut chunks = 0;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let error = burner
        .write(
            0x1_0000,
            &mut source,
            2048,
            0,
            &mut |_, _| {
                chunks += 1;
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            },
            &|| cancel.load(std::sync::atomic::Ordering::SeqCst),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert_eq!(chunks, 1);
}
