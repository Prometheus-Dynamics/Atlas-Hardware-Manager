//! Atlas's client against the real daemon session, over a socket pair in
//! place of the USB bulk endpoints, writing a file in place of the eMMC.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;

use atlas_flasher::{Flasher, FlasherError, ImageFormat, Progress, StreamTransport};
use atlas_flasher_proto::{Phase, RebootInto};
use flasher::board::Board;
use flasher::session::{self, Context, Outcome};
use sha2::{Digest, Sha256};

const EMMC: u64 = 8 << 20;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An image that compresses but isn't trivial.
fn image(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31) ^ (i >> 9)) as u8).collect()
}

struct Rig {
    flasher: Flasher,
    board: thread::JoinHandle<Outcome>,
    disk: PathBuf,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let disk = dir.path().join("mmcblk0");
    std::fs::write(&disk, vec![0xee; EMMC as usize]).unwrap();
    let (host_out, board_in) = UnixStream::pair().unwrap();
    let (board_out, host_in) = UnixStream::pair().unwrap();
    let context = Context {
        disk: disk.clone(),
        board: Board {
            serial: "10000000a317bcbe".into(),
            model: "Raze".into(),
            revision: Some("d04180".into()),
        },
        version: "test".into(),
        drop_caches: false,
    };
    let board = thread::spawn(move || session::serve(board_in, board_out, &context));
    host_out.set_nonblocking(true).unwrap();
    host_in.set_nonblocking(true).unwrap();
    let to_board = tokio::net::UnixStream::from_std(host_out).unwrap();
    let from_board = tokio::net::UnixStream::from_std(host_in).unwrap();
    Rig {
        flasher: Flasher::new(StreamTransport(to_board), from_board),
        board,
        disk,
        _dir: dir,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zstd_image_is_written_verified_and_the_board_reboots() {
    let mut rig = rig();
    let hello = rig
        .flasher
        .hello("atlas-test", Some("a317bcbe"))
        .await
        .unwrap();
    assert_eq!(hello.model, "Raze");
    assert_eq!(rig.flasher.info().await.unwrap().size, EMMC);

    let raw = image(5 << 20);
    let packed =
        ruzstd::encoding::compress_to_vec(&raw[..], ruzstd::encoding::CompressionLevel::Fastest);
    let sha = hex(&Sha256::digest(&packed));
    let mut seen: Vec<Progress> = Vec::new();
    let (bytes, written_sha) = rig
        .flasher
        .write(
            &packed[..],
            packed.len() as u64,
            Some(sha),
            ImageFormat::Zst,
            &mut |p| seen.push(p),
        )
        .await
        .unwrap();
    assert_eq!(bytes, raw.len() as u64);
    assert_eq!(written_sha, hex(&Sha256::digest(&raw)));
    assert!(seen.iter().any(|p| p.phase == Phase::Writing));

    let on_disk = std::fs::read(&rig.disk).unwrap();
    assert_eq!(&on_disk[..raw.len()], &raw[..]);
    assert_eq!(on_disk[raw.len()], 0xee, "past the image is left alone");

    let mut verifying = Vec::new();
    rig.flasher
        .verify(&mut |p| verifying.push(p))
        .await
        .unwrap();
    assert!(verifying.last().is_some_and(|p| p.fraction() == 1.0));

    rig.flasher.reboot().await.unwrap();
    assert_eq!(rig.board.join().unwrap(), Outcome::Reboot(RebootInto::Emmc));
}

#[tokio::test(flavor = "multi_thread")]
async fn another_board_is_refused() {
    let mut rig = rig();
    let error = rig
        .flasher
        .hello("atlas-test", Some("deadbeef"))
        .await
        .unwrap_err();
    assert!(matches!(error, FlasherError::WrongBoard { .. }), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupted_image_is_caught_and_the_link_stays_usable() {
    let mut rig = rig();
    let raw = image(1 << 20);
    let error = rig
        .flasher
        .write(
            &raw[..],
            raw.len() as u64,
            Some("00".repeat(32)),
            ImageFormat::Raw,
            &mut |_| {},
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("SHA-256"), "{error}");
    // Nothing verified after a failed write; the session goes on.
    assert!(rig.flasher.verify(&mut |_| {}).await.is_err());
    rig.flasher.hello("atlas-test", None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_bigger_than_the_emmc_is_refused_up_front() {
    let mut rig = rig();
    let raw = image((EMMC + 1) as usize);
    let error = rig
        .flasher
        .write(
            &raw[..],
            raw.len() as u64,
            None,
            ImageFormat::Raw,
            &mut |_| {},
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("eMMC"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_xz_image_is_decompressed_on_the_board() {
    let mut rig = rig();
    let raw = image(3 << 20);
    let mut packed = Vec::new();
    lzma_rs::xz_compress(&mut &raw[..], &mut packed).unwrap();
    let (bytes, sha) = rig
        .flasher
        .write(
            &packed[..],
            packed.len() as u64,
            None,
            ImageFormat::Xz,
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(bytes, raw.len() as u64);
    assert_eq!(sha, hex(&Sha256::digest(&raw)));
    rig.flasher.verify(&mut |_| {}).await.unwrap();
}
