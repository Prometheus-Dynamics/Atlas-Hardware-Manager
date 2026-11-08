//! The native boot_merger against a loader Radxa publishes, and against its
//! own parser.
//!
//! The byte-for-byte test needs `/mnt/ssd-samsung/atlas-blobs/merger-ref` (see
//! `merger-src/SOURCES.txt` there); without it, it says so and passes, so
//! CI machines without the blobs only run the self-contained tests.

use std::io;
use std::path::Path;

use atlas_socflash::rockchip::crc::rk_crc32;
use atlas_socflash::rockchip::loader::{EntryKind, Loader};
use atlas_socflash::rockchip::merge::{Ini, MergeError, ReleaseTime, merge, merge_dir, merge_full};
use atlas_socflash::rockchip::rc4::rc4_blocks;

const TIME: ReleaseTime = ReleaseTime {
    year: 2022,
    month: 8,
    day: 30,
    hour: 16,
    minute: 51,
    second: 13,
};

const INI: &str = "[CHIP_NAME]\nNAME=RK3588\n[VERSION]\nMAJOR=1\nMINOR=11\n\
[CODE471_OPTION]\nNUM=1\nPath1=bin/ddr.bin\nSleep=1\n\
[CODE472_OPTION]\nNUM=1\nPath1=bin/usbplug_v1.09.bin\n\
[LOADER_OPTION]\nNUM=2\nLOADER1=FlashData\nLOADER2=FlashBoot\n\
FlashData=bin/ddr.bin\nFlashBoot=bin/spl.bin\n\
[OUTPUT]\nPATH=out.bin\n[SYSTEM]\nNEWIDB=true\n[FLAG]\n471_RC4_OFF=true\nRC4_OFF=true\n";

fn parts(path: &str) -> io::Result<Vec<u8>> {
    match path {
        "bin/ddr.bin" => Ok(vec![0xd1; 3000]),
        "bin/usbplug_v1.09.bin" => Ok(vec![0xa2; 2048]),
        "bin/spl.bin" => Ok(vec![0x5b; 700]),
        _ => Err(io::Error::new(io::ErrorKind::NotFound, "no such part")),
    }
}

fn built(ini_text: &str) -> Result<Vec<u8>, MergeError> {
    merge(&Ini::parse(ini_text)?, TIME, parts)
}

#[test]
fn a_merged_loader_parses_back_with_its_offsets_delays_and_crc() {
    let file = built(INI).unwrap();
    let loader = Loader::parse(&file).unwrap();
    assert_eq!(loader.chip_number().as_deref(), Some("3588"));
    assert_eq!(loader.version, 0x010b);
    assert!(loader.rc4_disabled && !loader.signed);
    assert_eq!(loader.crc_polynomial, 0x04c1_0db7);
    let tail = file.len() - 4;
    let crc = u32::from_le_bytes(file[tail..].try_into().unwrap());
    assert_eq!(crc, rk_crc32(&file[..tail], 0x04c1_0db7));

    let layout: Vec<_> = loader
        .entries
        .iter()
        .map(|e| (e.kind, e.name.as_str(), e.size, e.delay_ms))
        .collect();
    assert_eq!(
        layout,
        [
            (EntryKind::Ddr, "UsbHead", 2048, 1),
            (EntryKind::Ddr, "ddr", 4096, 1),
            (EntryKind::Usbplug, "usbplug_v1", 2048, 0),
            (EntryKind::Loader, "FlashHead", 2048, 0),
            (EntryKind::Loader, "FlashData", 4096, 0),
            (EntryKind::Loader, "FlashBoot", 2048, 0),
        ]
    );
    // The data follows the table in entry order, with no gaps.
    let mut at = 102 + 6 * 57;
    for entry in &loader.entries {
        assert_eq!(entry.offset as usize, at);
        at += entry.size as usize;
    }
    assert_eq!(at, tail);
    // 471 and 472 stages are padded plain; loader stages are scrambled.
    let ddr = loader.entries(EntryKind::Ddr).nth(1).unwrap().data(&file);
    assert_eq!(&ddr[..3000], &[0xd1; 3000][..]);
    assert!(ddr[3000..].iter().all(|b| *b == 0));
    let spl = loader.loader_entry("FlashBoot").unwrap().data(&file);
    let mut plain = vec![0x5b; 700];
    plain.resize(2048, 0);
    assert_eq!(rc4_blocks(spl), plain);
}

#[test]
fn rc4_off_only_changes_the_header_flag_and_the_stages_follow_471_rc4_off() {
    let on = built(&INI.replace("RC4_OFF=true\n", "RC4_OFF=false\n")).unwrap();
    // "471_RC4_OFF=true" contains "RC4_OFF=true\n": both were replaced.
    let loader = Loader::parse(&on).unwrap();
    assert!(!loader.rc4_disabled);
    let ddr = loader.entries(EntryKind::Ddr).nth(1).unwrap().data(&on);
    assert_ne!(&ddr[..16], &[0xd1; 16][..]);

    let header_only = INI.replace("\nRC4_OFF=true", "\nRC4_OFF=false");
    let flagged = built(&header_only).unwrap();
    let off = built(INI).unwrap();
    assert_eq!(flagged.len(), off.len());
    assert_eq!(flagged[44], 0);
    assert_eq!(off[44], 1);
    let loader = Loader::parse(&flagged).unwrap();
    let entry = loader.entries(EntryKind::Ddr).nth(1).unwrap();
    assert_eq!(&entry.data(&flagged)[..3000], &[0xd1; 3000][..]);
}

#[test]
fn bad_input_is_refused_with_a_message() {
    let refused = |ini: String| built(&ini).unwrap_err().to_string();
    assert!(refused(INI.replace("bin/spl.bin", "bin/gone.bin")).contains("bin/gone.bin"));
    assert!(refused(INI.replace("NAME=RK3588", "NAME=RK35888")).contains("too long"));
    assert!(refused(INI.replace("MINOR=11", "MINOR=300")).contains("version"));
    assert!(refused(INI.replace("[VERSION]", "[VERSIONS]")).contains("unknown section"));
    assert!(refused(INI.replace("Path1=bin/ddr.bin", "Pat=bin/ddr.bin")).contains("Path1"));
    assert!(refused(INI.replace("MAJOR=1\n", "MAJOR=x\n")).contains("not a number"));
    assert!(refused(INI.replace("[OUTPUT]\nPATH=out.bin\n", "")).contains("OUTPUT"));
    assert!(refused(INI.replace("NEWIDB=true", "NEWIDB=false")).contains("NEWIDB"));
    assert!(refused(INI.replace("FlashBoot=bin/spl.bin\n", "")).contains("FlashBoot"));
    assert!(refused(format!("{INI}ALIGN=8\n[FLAG2]\n")).contains("unknown section"));
    assert!(refused(format!("{INI}[LOADER3_PARAM]\nLOAD_ADDR=0x1\n")).contains("only 2 loaders"));
    assert!(refused(format!("{INI}[LOADER1_PARAM]\nLOAD=0x1\n")).contains("LOAD"));
    assert!(refused(format!("{INI}[BOOT1_PARAM]\nWORD_8=0x5\n")).contains("WORD_8"));
    assert!(refused("garbage".into()).contains("line 1"));
    let empty = merge(&Ini::parse(INI).unwrap(), TIME, |_| Ok(Vec::new()));
    assert!(matches!(empty, Err(MergeError::EmptyPart(_))));
}

#[test]
fn align_params_and_the_idblock_shape_the_output() {
    let plain = merge_full(&Ini::parse(INI).unwrap(), TIME, parts).unwrap();
    assert!(plain.idblock.is_none());
    let text = format!(
        "{INI}[LOADER1_PARAM]\nLOAD_ADDR=0x3FFC0000\nFLAG=0x2\n[BOOT1_PARAM]\nWORD_2=0x100\n"
    )
    .replace("[SYSTEM]\n", "[SYSTEM]\nALIGN=8\n")
    .replace("[FLAG]\n", "[FLAG]\nCREATE_IDB=true\nCRC_OFF=true\n");
    let merged = merge_full(&Ini::parse(&text).unwrap(), TIME, parts).unwrap();
    let file = merged.loader;
    let loader = Loader::parse(&file).unwrap();
    let sizes: Vec<_> = loader.entries.iter().map(|e| e.size).collect();
    assert_eq!(sizes, [2048, 4096, 2048, 4096, 4096, 4096]);
    assert_eq!(file[46], 1, "CRC_OFF");
    let idblock = merged.idblock.unwrap();
    assert_eq!(idblock.len(), 4096 * 3);
    assert_eq!(&idblock[..4], b"RKNS");
    // WORD_2 at 96; the first image at sector 8 with LOAD_ADDR and FLAG.
    assert_eq!(&idblock[96..100], &0x100u32.to_le_bytes());
    assert_eq!(&idblock[120..124], &((8u32 << 16) + 8).to_le_bytes());
    assert_eq!(&idblock[124..132], &[0, 0, 0xfc, 0x3f, 2, 0, 0, 0]);
    assert_eq!(&idblock[4096..4096 + 4], &[0xd1; 4]);
}

#[test]
fn the_unix_time_helper_matches_known_dates() {
    let t = ReleaseTime::from_unix_utc(1_661_878_273);
    assert_eq!((t.year, t.month, t.day), (2022, 8, 30));
    assert_eq!((t.hour, t.minute, t.second), (16, 51, 13));
    let leap = ReleaseTime::from_unix_utc(951_782_400);
    assert_eq!((leap.year, leap.month, leap.day), (2000, 2, 29));
}

const REF_DIR: &str = "/mnt/ssd-samsung/atlas-blobs/merger-ref";

/// Radxa's rk3588_spl_loader_v1.08.111.bin (radxa/rkbin fc44f94) against the
/// loader built from rockchip-linux/rkbin 35a0b2d's RK3588MINIALL.ini and
/// the three parts it names. The only input not in the ini or the parts is
/// the header's release time, which the C takes from the clock: the
/// reference's own (2022-08-30 16:51:13) is passed in.
#[test]
fn rebuilds_radxas_published_rk3588_loader_byte_for_byte() {
    let root = Path::new(REF_DIR);
    let parts = root.join("rk3588-rkbin-35a0b2d");
    let reference = root.join("radxa-rkbin-fc44f94/rk3588_spl_loader_v1.08.111.bin");
    let (Ok(text), Ok(published)) = (
        std::fs::read_to_string(parts.join("RKBOOT/RK3588MINIALL.ini")),
        std::fs::read(&reference),
    ) else {
        tracing::warn!("{REF_DIR} is not here: the byte-for-byte check was skipped");
        return;
    };
    let ini = Ini::parse(&text).unwrap();
    assert_eq!(ini.output, "rk3588_spl_loader_v1.08.111.bin");
    let ours = merge_dir(&ini, TIME, &parts).unwrap();
    let first = ours.iter().zip(&published).position(|(a, b)| a != b);
    assert_eq!(
        (first, ours.len()),
        (None, published.len()),
        "first differing offset"
    );
    assert_eq!(ours, published);
}
