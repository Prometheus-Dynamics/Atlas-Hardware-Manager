//! The native boot_merger against the loaders Gaia built with Rockchip's
//! boot_merger 1.35 / 1.38 (`/mnt/ssd-samsung/gaia/boards/<board>/loader/`),
//! from the ini and parts Gaia built them from.
//!
//! The ini and the reference are read from Gaia's loader directory; the
//! vendor parts from `atlas-blobs/<device>/` (fetched at the rkbin SHAs in
//! `devices/<device>/fetch.lock`). Without those files a test says so and
//! passes. The release time is the reference's own (header bytes 0x0E-0x14)
//! and the merge version is read from it too; nothing else is taken from it.

use std::path::{Path, PathBuf};

use atlas_socflash::rockchip::merge::{Ini, ReleaseTime, merge_full};
use sha2::{Digest, Sha256};

const GAIA: &str = "/mnt/ssd-samsung/gaia/boards";
const BLOBS: &str = "/mnt/ssd-samsung/atlas-blobs";

struct Board {
    /// Gaia's board directory and the blob directory of the parts.
    gaia: &'static str,
    blobs: &'static str,
    ini: &'static str,
    loader: &'static str,
    /// sha256 of the loader in Gaia's REPORT.md.
    loader_sha256: &'static str,
    /// The idblock file beside the loader and its REPORT.md sha256.
    idblock: Option<(&'static str, &'static str)>,
    /// A part saved beside the loader that replaces the ini's FlashBoot.
    spl: Option<(&'static str, &'static str)>,
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn time_of(reference: &[u8]) -> ReleaseTime {
    ReleaseTime {
        year: u16::from_le_bytes([reference[14], reference[15]]),
        month: reference[16],
        day: reference[17],
        hour: reference[18],
        minute: reference[19],
        second: reference[20],
    }
}

/// The first offset where two files differ (or where one ends).
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or((a.len() != b.len()).then_some(a.len().min(b.len())))
}

fn check(board: &Board) {
    let dir = PathBuf::from(GAIA).join(board.gaia).join("loader");
    let blobs = PathBuf::from(BLOBS).join(board.blobs);
    let (Ok(text), Ok(reference)) = (
        std::fs::read_to_string(dir.join(board.ini)),
        std::fs::read(dir.join(board.loader)),
    ) else {
        tracing::warn!("{} is not here: {} was skipped", dir.display(), board.gaia);
        return;
    };
    assert_eq!(sha256(&reference), board.loader_sha256, "REPORT.md guard");
    let mut ini = Ini::parse(&text).unwrap();
    assert_eq!(ini.output, board.loader);
    ini.merger_version = u32::from_le_bytes(reference[10..14].try_into().unwrap());
    let spl = board.spl.map(|(file, sha)| {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        assert_eq!(sha256(&bytes), sha, "REPORT.md guard for {file}");
        assert!(ini.set_flash_boot(file));
        (file, bytes)
    });
    let read = |path: &str| -> std::io::Result<Vec<u8>> {
        if let Some((file, bytes)) = &spl
            && path == *file
        {
            return Ok(bytes.clone());
        }
        let name = Path::new(path).file_name().unwrap();
        std::fs::read(blobs.join(name))
    };
    let merged = merge_full(&ini, time_of(&reference), read).unwrap();
    let ours = merged.loader;
    assert_eq!(
        (first_difference(&ours, &reference), ours.len()),
        (None, reference.len()),
        "{}: first differing offset (sha256 ours {})",
        board.gaia,
        sha256(&ours)
    );
    assert_eq!(sha256(&ours), board.loader_sha256);
    if let Some((file, sha)) = board.idblock {
        let Ok(idblock) = std::fs::read(dir.join(file)) else {
            tracing::warn!("{file} is not here: the idblock check was skipped");
            return;
        };
        assert_eq!(sha256(&idblock), sha, "REPORT.md guard for {file}");
        let ours = merged.idblock.expect("CREATE_IDB builds an idblock");
        assert_eq!(first_difference(&ours, &idblock), None, "idblock");
        assert_eq!(sha256(&ours), sha);
    } else {
        assert!(merged.idblock.is_none(), "no CREATE_IDB, no idblock");
    }
}

#[test]
fn rock_5c_rk3588_loader_v1_18_113() {
    check(&Board {
        gaia: "rock-5c",
        blobs: "radxa-rock-5c",
        ini: "RK3588MINIALL.ini",
        loader: "rk3588_spl_loader_v1.18.113.bin",
        loader_sha256: "7aacdfc4ba8e594e8cc8731a81d60a800f60757df1c190a173f5a14577b6cacc",
        idblock: None,
        spl: None,
    });
}

#[test]
fn rock_4d_rk3576_loader_and_idblock_v1_13_109() {
    check(&Board {
        gaia: "rock-4d",
        blobs: "radxa-rock-4d",
        ini: "RK3576MINIALL.ini",
        loader: "rk3576_loader_v1.13.109.bin",
        loader_sha256: "07583b8608c1068218c6e00bafa093375ee89eb16356a52ce0eb3c9dde2a032a",
        idblock: Some((
            "rk3576_idblock_v1.13.109.img",
            "5bc5031f334ea10fced8bdc742248d669c92fd2b728760dbf1cb708870bfd037",
        )),
        spl: None,
    });
}

#[test]
fn zero_3w_rk356x_loader_v1_23_114() {
    check(&Board {
        gaia: "zero-3w",
        blobs: "radxa-zero-3w",
        ini: "RK3566MINIALL.ini",
        loader: "rk356x_spl_loader_v1.23.114.bin",
        loader_sha256: "356445f4bacfbc8129c6d2849cccb5b34e06fabf83ffa30cc057f059c10b2848",
        idblock: None,
        spl: None,
    });
}

#[test]
fn luckfox_pico_pro_rv1106_loader_and_idblock_with_the_sdk_spl() {
    check(&Board {
        gaia: "luckfox-pico-pro",
        blobs: "luckfox-pico-pro",
        ini: "RV1106MINIALL.ini",
        loader: "rv1106_download_v1.15.108.bin",
        loader_sha256: "c3adefa3b2a0ea9ac7c810fd54c89c61463009a9b3e887d00876853bdc1784be",
        idblock: Some((
            "rv1106_idblock_v1.15.102.img",
            "39ddf5614d49fd0072c5e3afa6b7c5c65c425a45e65bb1256bdcbdb6703d5154",
        )),
        spl: Some((
            "u-boot-spl.bin",
            "825a7a8d97eabc41105f8c70aeeff7326983c2da21eb77b8a1e50393ba2246a6",
        )),
    });
}
