use super::*;

#[test]
fn the_chip_decides_before_anything_is_written() {
    let rk3576 = || {
        let mut info = b"6753".to_vec();
        info.resize(16, 0);
        Ok(info)
    };
    let rk3588 = || {
        let mut info = b"8853".to_vec();
        info.resize(16, 0);
        Ok(info)
    };
    let unreadable = || Ok(vec![0xff; 16]);
    let lost = || Err("timed out".to_string());
    let rk3576_tags = vec!["3576".to_string()];
    let rk3588_tags = vec!["3588".to_string()];
    assert_eq!(
        chip_verdict(rk3576(), Some("RK3576"), Some(&rk3576_tags), true).unwrap(),
        "chip: 3576"
    );
    assert_eq!(
        chip_verdict(rk3588(), Some("RK3588S2"), Some(&rk3588_tags), false).unwrap(),
        "chip: 3588"
    );
    // Another chip stops the flash, guessed or not.
    for required in [true, false] {
        let why = chip_verdict(rk3588(), Some("RK3576"), Some(&rk3576_tags), required).unwrap_err();
        assert!(why.contains("nothing was written"), "{why}");
    }
    // Can't tell: goes on for a confirmed PID, stops for a guessed one.
    assert!(chip_verdict(unreadable(), Some("RK3576"), Some(&rk3576_tags), false).is_ok());
    assert!(chip_verdict(lost(), Some("RK3576"), Some(&rk3576_tags), false).is_ok());
    assert!(chip_verdict(unreadable(), Some("RK3576"), Some(&rk3576_tags), true).is_err());
    assert!(chip_verdict(lost(), Some("RK3576"), Some(&rk3576_tags), true).is_err());
    assert!(chip_verdict(rk3576(), None, None, true).is_err());
}

#[test]
fn the_chip_is_the_family_list_not_a_substring_of_the_soc() {
    // RV1126B's family accepts 110F (rkbin's RV110F) and 1126; reversed
    // ASCII is how the reply carries them.
    let reply = |tag: &str| {
        let mut info: Vec<u8> = tag.bytes().rev().collect();
        info.resize(16, 0);
        Ok(info)
    };
    let aura = vec!["110F".to_string(), "1126".to_string()];
    assert!(chip_verdict(reply("110F"), Some("RV1126B"), Some(&aura), true).is_ok());
    assert!(chip_verdict(reply("1126"), Some("RV1126B"), Some(&aura), true).is_ok());
    // An RV1106 loader names 1106: refused for the RV1126B, even though
    // neither string contains the other's tag.
    let why = chip_verdict(reply("1106"), Some("RV1126B"), Some(&aura), false).unwrap_err();
    assert!(why.contains("nothing was written"), "{why}");
    // A SoC the family names no list for refuses any readable chip.
    assert!(chip_verdict(reply("110F"), Some("RV1126B"), None, false).is_err());
}

/// Gaia's real outputs, when they're on this machine (skipped otherwise).
const GAIA: &str = "/mnt/ssd-samsung/gaia/work/.gaia/examples";

#[test]
fn gaias_k230d_image_has_valid_spl_heads() {
    let image = Path::new(GAIA).join("k230d-zero/out/images/sdcard.img");
    if !image.is_file() {
        return;
    }
    let lines = k230_heads(&image).unwrap();
    assert_eq!(lines.len(), 2);
    assert!(
        lines.iter().all(|line| line.contains("SHA-256 matches")),
        "{lines:?}"
    );
}

#[test]
fn gaias_luckfox_output_is_a_partition_set() {
    let dir = Path::new(GAIA).join("luckfox-pico-pro/out/images");
    if !dir.join("env.img").is_file() {
        return;
    }
    let parts = crate::layout::partition_set(&dir.join("ubi.img"))
        .unwrap()
        .unwrap();
    let named: Vec<(&str, u64, bool)> = parts
        .iter()
        .map(|planned| {
            (
                planned.part.name.as_str(),
                planned.part.offset,
                planned.image.is_some(),
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            ("env", 0, true),
            ("idblock", 256 << 10, true),
            ("uboot", 512 << 10, true),
            ("boot", 1 << 20, true),
            ("boot_b", 7 << 20, false),
            ("ubi", 13 << 20, true),
        ]
    );
    // The UBI partition's image is UBI from its first byte, and reads.
    let ubi = dir.join("ubi.img");
    assert_eq!(find_ubi_start(&ubi, 128 << 10).unwrap(), Some(0));
    let image = atlas_ubi::read_image(&mut File::open(&ubi).unwrap(), 128 << 10).unwrap();
    let names: Vec<&str> = image.volumes.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["rootfs_a", "rootfs_b", "data"]);
}
