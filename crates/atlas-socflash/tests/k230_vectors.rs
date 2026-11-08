//! Byte vectors for the K230 USB code, derived by hand from Canaan's
//! k230_flash at commit 33a75f0 (BSD-2-Clause). In every comment "U:n" is
//! a line of burner_uboot.cpp, "B:n" of burner_brom.cpp, "P:n" of
//! probe_device.cpp, "H:n" of kburn_k230.h, "K:n" of kburn.h and "F:n" of
//! k230_flash.cpp. Nothing here was captured from a board.

mod k230_common;

use atlas_socflash::k230::bootrom::{self, Stage};
use atlas_socflash::k230::kburn::{Burner, Medium, cmd, packet};
use atlas_socflash::{Script, Step};
use k230_common::*;
use sha2::{Digest, Sha256};

#[test]
fn command_packets_match_the_c_layout() {
    // U:46-58: 60 bytes, header 6, data 54. Command numbers U:16-33.
    assert_eq!(packet(cmd::NOP, &[]).to_vec(), pad("0000 0000 0000"));
    // U:503-515: DEV_PROBE data = [medium, 0xFF], size 2. SD is 2 (K:41),
    // SPI_NAND 3 (K:42).
    assert_eq!(
        packet(cmd::PROBE, &[2, 0xff]).to_vec(),
        pad("1000 0000 0200 02ff")
    );
    assert_eq!(
        packet(cmd::PROBE, &[3, 0xff]).to_vec(),
        pad("1000 0000 0200 03ff")
    );
    // U:545-550: DEV_GET_INFO, no data.
    assert_eq!(packet(cmd::GET_INFO, &[]).to_vec(), pad("1100 0000 0000"));
    // U:570-600: ERASE_LBA, {offset, size} as two u64, data_size 16.
    assert_eq!(
        packet(cmd::ERASE, &words(&[0x10_0000, 0x4_0000])).to_vec(),
        pad("2000 0000 1000 0000100000000000 0000040000000000")
    );
    // U:633-690: WRITE_LBA, loader version >= 1: {offset, size, max, flag},
    // data_size 32; below 1: the first three, data_size 24.
    assert_eq!(
        packet(cmd::WRITE, &words(&[0x10_0000, 0x600, 0x2000, 0])).to_vec(),
        pad("2100 0000 2000 0000100000000000 0006000000000000 0020000000000000 0000000000000000")
    );
    assert_eq!(
        packet(cmd::WRITE, &words(&[0x10_0000, 0x600, 0x2000])).to_vec(),
        pad("2100 0000 1800 0000100000000000 0006000000000000 0020000000000000")
    );
    // U:781-798: READ_LBA {offset, size}.
    assert_eq!(
        packet(cmd::READ, &words(&[0x200, 0x400])).to_vec(),
        pad("2300 0000 1000 0002000000000000 0004000000000000")
    );
    // U:369-380, 395: VERIFY_LBA {offset, size}.
    assert_eq!(
        packet(cmd::VERIFY, &words(&[0, 0x600])).to_vec(),
        pad("2500 0000 1000 0000000000000000 0006000000000000")
    );
    // U:484-501: REBOOT, data_size 8, the u64 0x52626F74 little endian.
    assert_eq!(
        packet(cmd::REBOOT, &0x5262_6f74u64.to_le_bytes()).to_vec(),
        pad("0100 0000 0800 746f625200000000")
    );
    // U:26: WRITE_LBA_CHUNK (0x22) and U:29: READ_LBA_CHUNK (0x24) exist as
    // numbers; the C tool never sends 0x22 (the data goes out raw).
    assert_eq!((cmd::WRITE_CHUNK, cmd::READ_CHUNK), (0x22, 0x24));
}

#[tokio::test]
async fn reboot_sends_the_mark_and_expects_no_reply() {
    // U:484-501: one write, no read.
    let mut script = Script::new(vec![Step::BulkOut(pad("0100 0000 0800 746f625200000000"))]);
    Burner::new(&mut script).reboot().await.unwrap();
    assert!(script.finished());
}

#[tokio::test]
async fn brom_requests_split_the_address_high_and_low() {
    // B:73-93 and B:16-37: SET_DATA_ADDRESS (1) and PROG_START (4), out,
    // vendor, device; wValue = address >> 16, wIndex = address & 0xffff
    // (H:16-17), no data. The default address is H:31 / F:403.
    for (address, value, index) in [
        (0x8036_0000u32, 0x8036u16, 0x0000u16),
        (0x8030_0000, 0x8030, 0x0000),
        (0x8036_1234, 0x8036, 0x1234),
    ] {
        let request = |request| Step::ControlOut {
            request,
            value,
            index,
            data: Vec::new(),
        };
        let mut script = Script::new(vec![request(1), request(4)]);
        bootrom::run_loader(&mut script, &[], address)
            .await
            .unwrap();
        assert!(script.finished(), "{address:#x}");
    }
    assert_eq!(bootrom::DEFAULT_LOAD_ADDRESS, 0x8036_0000);
    // F:109: the command line takes 0x80300000..=0x80400000.
    assert!(bootrom::LOAD_ADDRESS_RANGE.contains(&0x8030_0000));
    assert!(!bootrom::LOAD_ADDRESS_RANGE.contains(&0x802f_ffff));
}

#[tokio::test]
async fn the_loader_chunking_matches_the_c_loop() {
    // B:119-156: pages = ceil(size/1000); a page is 1000 bytes unless
    // offset + 1000 > size, then size % 1000.
    for size in [0usize, 1, 999, 1000, 1001, 1999, 2000, 2500, 65_537] {
        let loader: Vec<u8> = (0..size).map(|i| (i * 7) as u8).collect();
        let mut steps = vec![Step::ControlOut {
            request: 1,
            value: 0x8036,
            index: 0,
            data: Vec::new(),
        }];
        let pages = size.div_ceil(1000);
        for page in 0..pages {
            let offset = page * 1000;
            let chunk = if offset + 1000 > size {
                size % 1000
            } else {
                1000
            };
            steps.push(Step::BulkOut(loader[offset..offset + chunk].to_vec()));
        }
        steps.push(Step::ControlOut {
            request: 4,
            value: 0x8036,
            index: 0,
            data: Vec::new(),
        });
        let mut script = Script::new(steps);
        bootrom::run_loader(&mut script, &loader, 0x8036_0000)
            .await
            .unwrap();
        assert!(script.finished(), "{size} bytes");
    }
}

#[tokio::test]
async fn the_cpu_info_strings_tell_the_stage() {
    // P:9-23 (IN, vendor, device, request 0, wValue 0, wIndex 0, 32
    // bytes); P:49 "Uboot Stage for K230" and P:55 "K230".
    let info = |reply: &[u8]| Step::ControlIn {
        request: 0,
        value: 0,
        index: 0,
        length: 32,
        reply: reply.to_vec(),
    };
    let mut script = Script::new(vec![
        info(b"K230"),
        info(b"Uboot Stage for K230"),
        info(b"Uboot Stage for K230\0\0\0\0\0\0\0\0\0\0\0\0"),
        info(b"K230D"),
    ]);
    assert_eq!(bootrom::stage(&mut script).await.unwrap(), Stage::BootRom);
    assert_eq!(bootrom::stage(&mut script).await.unwrap(), Stage::Burner);
    assert_eq!(bootrom::stage(&mut script).await.unwrap(), Stage::Burner);
    assert_eq!(
        bootrom::stage(&mut script).await.unwrap(),
        Stage::Unknown("K230D".into())
    );
}

#[tokio::test]
async fn a_whole_session_in_c_order() {
    // Sync (U:872-897), probe (U:503), info (U:545), erase (U:570), write
    // (U:633-780, aligned up to the block size, zero padded: U:951, U:1002),
    // verify (U:369, loader version 2), reboot (U:484).
    let image: Vec<u8> = (0..1300u32).map(|i| (i * 31) as u8).collect();
    let mut padded = image.clone();
    padded.resize(1536, 0);
    let mut steps = Vec::new();
    ready(&mut steps, 2, 1024, 512);
    steps.extend(ok(0x20, &words(&[0x10_0000, 0x1_0000])));
    // WRITE_LBA start {offset, aligned size, max, 0}, reply, the raw
    // chunks (no ZLP: each equals what the burner expects, U:722-733), the
    // end reply, then the NOP.
    steps.push(out(0x21, &words(&[0x10_0000, 1536, 0x2000, 0])));
    steps.push(rep(0x21, 1, &[]));
    steps.push(Step::BulkOut(padded[..1024].to_vec()));
    steps.push(Step::BulkOut(padded[1024..].to_vec()));
    steps.push(rep(0x21, 1, &[]));
    steps.push(out(0, &[]));
    steps.push(rep(0, 1, &[]));
    // U:369-432: the reply is digest, verified bytes, elapsed ms.
    let digest = Sha256::digest(&image);
    let mut verify = digest.to_vec();
    verify.extend(words(&[1300, 42]));
    steps.push(out(0x25, &words(&[0x10_0000, 1300])));
    steps.push(rep(0x25, 1, &verify));
    steps.push(Step::BulkOut(pad("0100 0000 0800 746f625200000000")));
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    assert_eq!(burner.loader_version(), 2);
    burner.erase(0x10_0000, 0x1_0000).await.unwrap();
    let sent = burner
        .write(
            0x10_0000,
            &mut &image[..],
            1300,
            0x2000,
            &mut |_, _, _| {},
            &|| false,
        )
        .await
        .unwrap();
    let digest_hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(sent, digest_hex);
    assert_eq!(burner.sha256(0x10_0000, 1300).await.unwrap(), sent);
    burner.reboot().await.unwrap();
    assert!(script.finished());
}

#[tokio::test]
async fn an_old_loader_gets_three_words_and_no_verify() {
    // U:633-690: loader_version < 1 sends 3 u64 (cfg_size 24); U:381-388:
    // verify below version 2 is refused before anything is sent.
    let mut steps = Vec::new();
    ready(&mut steps, 0, 1024, 512);
    steps.push(out(0x21, &words(&[0, 512, 0])));
    steps.push(rep(0x21, 1, &[]));
    steps.push(Step::BulkOut(vec![7; 512]));
    steps.push(rep(0x21, 1, &[]));
    steps.push(out(0, &[]));
    steps.push(rep(0, 1, &[]));
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    burner
        .write(0, &mut &[7u8; 512][..], 512, 0, &mut |_, _, _| {}, &|| {
            false
        })
        .await
        .unwrap();
    let error = burner.sha256(0, 512).await.unwrap_err();
    assert!(error.to_string().contains("verification"), "{error}");
    assert!(script.finished());
}

#[tokio::test]
async fn a_short_chunk_that_fills_its_packet_ends_with_a_zero_length_packet() {
    // U:711-733: chunk = out_chunk rounded down to the block (U:951); a
    // chunk shorter than min(out_chunk, remaining) whose size is a multiple
    // of the endpoint's max packet (512 here) is followed by a ZLP.
    let image = vec![9u8; 1024];
    let mut steps = Vec::new();
    ready(&mut steps, 2, 1000, 512);
    steps.push(out(0x21, &words(&[0, 1024, 0, 0])));
    steps.push(rep(0x21, 1, &[]));
    steps.push(Step::BulkOut(image[..512].to_vec()));
    steps.push(Step::BulkOut(Vec::new())); // 512 < min(1000, 1024)
    steps.push(Step::BulkOut(image[512..].to_vec())); // 512 == min(1000, 512)
    steps.push(rep(0x21, 1, &[]));
    steps.push(out(0, &[]));
    steps.push(rep(0, 1, &[]));
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    burner
        .write(0, &mut &image[..], 1024, 0, &mut |_, _, _| {}, &|| false)
        .await
        .unwrap();
    assert!(script.finished());
}

#[tokio::test]
async fn info_decodes_the_bit_field_word() {
    // H:46-56: timeout_ms:32, wp:8, type:7, valid:1 in one u64.
    let mut steps = sync(1);
    steps.extend(probe(2, 512, 512));
    steps.extend(info(
        8 << 30,
        512,
        1 << 20,
        30000 | (1 << 32) | (2 << 40) | VALID,
    ));
    steps.extend(info(8 << 30, 512, 1 << 20, 30000));
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    let second = burner.info().await.unwrap();
    assert_eq!(
        (second.capacity, second.block_size, second.erase_size),
        (8 << 30, 512, 1 << 20)
    );
    assert!(!second.valid && !second.write_protected);
    let mut steps = sync(1);
    steps.extend(probe(2, 512, 512));
    steps.extend(info(
        8 << 30,
        512,
        1 << 20,
        30000 | (1 << 32) | (2 << 40) | VALID,
    ));
    let mut script = Script::new(steps);
    let mut burner = Burner::new(&mut script);
    burner.probe(Medium::Sd).await.unwrap();
    let first = burner.info().await.unwrap();
    assert!(first.valid && first.write_protected);
}

#[tokio::test]
async fn reads_come_back_as_whole_blocks_cut_to_the_request() {
    // U:1076-1150, U:781-857: READ_LBA {aligned offset, aligned size}, then
    // packets of 6 + n bytes (cmd 0x8024, result 1, size n), then a final
    // READ_LBA_CHUNK reply.
    let medium: Vec<u8> = (0..512u32).map(|i| (i * 3) as u8).collect();
    let chunk = |data: &[u8]| Step::BulkIn {
        max: 6 + data.len(),
        reply: pkt(0x8024, 1, data)[..6 + data.len()].to_vec(),
    };
    let mut steps = sync(1);
    steps.extend(probe(2, 1024, 400));
    steps.extend(info(1 << 20, 512, 4096, VALID));
    steps.extend(ok(0x23, &words(&[0, 512])));
    steps.push(chunk(&medium[..400]));
    steps.push(chunk(&medium[400..]));
    steps.push(rep(0x24, 1, &[]));
    let mut script = Script::new(steps);
    let mut burner = start(&mut script).await;
    let read = burner.read(100, 300).await.unwrap();
    assert_eq!(read, medium[100..400]);
    assert!(script.finished());
}

#[test]
fn the_record_agrees_with_the_c_source() {
    // The record's facts: flash.load_address 0x80360000 is the tools' own
    // default (kburn_k230.h:31, k230_flash.cpp:403) and sits inside the range
    // the CLI accepts (k230_flash.cpp:109-121); storage "sd" is medium 2
    // (kburn.h:39-44 KBURN_MEDIUM_SDCARD).
    let record = include_str!("../../../devices/bpi-canmv-k230d-zero/manifest.json");
    assert!(record.contains("\"load_address\": \"0x80360000\""));
    assert!(record.contains("\"storage\": \"sd\""));
    assert!(bootrom::LOAD_ADDRESS_RANGE.contains(&0x8036_0000));
    assert_eq!(bootrom::DEFAULT_LOAD_ADDRESS, 0x8036_0000);
    for (name, code) in [
        ("emmc", 1u8),
        ("sd", 2),
        ("spi-nand", 3),
        ("spi-nor", 4),
        ("otp", 5),
    ] {
        assert_eq!(Medium::from_name(name).unwrap() as u8, code, "{name}");
    }
    assert_eq!(Medium::from_name("nand"), None);
    // kburn.h:205-206.
    assert_eq!(
        (
            atlas_socflash::k230::VENDOR_ID,
            atlas_socflash::k230::PRODUCT_ID
        ),
        (0x29f1, 0x0230)
    );
}

#[test]
fn the_firmware_head_check_rejects_what_the_c_tool_never_looks_at() {
    // UNCONFIRMED by the C source (it has no firmware-head code): the
    // layout is the SDK's. These pin the behaviour only.
    use atlas_socflash::k230::firmware::{BODY_OFFSET, Head, MAGIC, check};
    let body = [5u8; 32];
    let mut head = vec![0u8; BODY_OFFSET];
    head[..4].copy_from_slice(MAGIC);
    head[4..8].copy_from_slice(&32u32.to_le_bytes());
    head[12..44].copy_from_slice(&Sha256::digest(body));
    head.extend_from_slice(&body);
    assert_eq!(
        check(&head),
        Head::Valid {
            length: 32,
            encryption: 0
        }
    );
    assert!(matches!(check(&head[..head.len() - 1]), Head::Damaged(_)));
    assert_eq!(check(&head[..100]), Head::Missing);
    head[0] = b'k';
    assert_eq!(check(&head), Head::Missing);
}
