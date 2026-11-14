//! The five boot files a slot carries (Lemnos I5i8), checked on the simulated board's own
//! `verify`, the install's check. A missing one is refused with the board's words; a sixth entry
//! is refused by name with this board's words until Lemnos I5i9 lands.

use atlas_sim_node::bundle::{self, BootRule, Verified};
use atlas_sim_node::slots::{Status, code};

const CMDLINE: &[u8] = b"console=ttyS0 os.src.rpi-boot-mode-1=/soc@107c000000/mmc@fff000\n";

fn verify(members: &[(String, Vec<u8>)]) -> Result<Verified, Status> {
    bundle::verify(
        &bundle::tar_of(members),
        &[],
        "simulated",
        &BootRule::default(),
    )
}

/// A bundle of `boot` as its five boot files (or as given), with a manifest listing `listed`.
fn bundle_listing(listed: &[(&str, &[u8])], carried: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
    let rootfs = b"rootfs of five\n".repeat(64);
    let manifest = bundle::manifest("five", "2.0.0", &rootfs, listed);
    let mut members = vec![
        ("slot.json".to_string(), manifest.to_string().into_bytes()),
        ("rootfs.erofs".to_string(), rootfs),
    ];
    members.extend(
        carried
            .iter()
            .map(|(path, bytes)| (format!("boot/{path}"), bytes.to_vec())),
    );
    members
}

#[test]
fn the_board_table_is_the_writers_table() {
    assert_eq!(bundle::BOOT_FILES, atlas_bundle::BOOT_FILES);
    assert_eq!(bundle::BOOT_FILES.len(), 5);
}

#[test]
fn a_five_file_bundle_installs_and_lists_its_five_boot_files() {
    let boot = bundle::boot_files(CMDLINE);
    let members = bundle_listing(&boot, &boot);
    let verified = verify(&members).expect("accepted");
    assert_eq!(verified.boot.len(), 5);
    assert_eq!(
        verified.payloads.iter().filter(|size| **size > 0).count(),
        5
    );
}

#[test]
fn a_missing_boot_file_the_manifest_does_not_list_is_refused_with_the_boards_words() {
    // Each name, left out of slot.json and of the members: the board's refusal at install.rs:756.
    for index in 0..5 {
        let full = bundle::boot_files(CMDLINE);
        let name = full[index].0;
        let listed: Vec<(&str, &[u8])> = full
            .iter()
            .copied()
            .filter(|(path, _)| *path != name)
            .collect();
        let error = verify(&bundle_listing(&listed, &listed)).expect_err("refused");
        assert_eq!(error.code, code::INVALID_ARGUMENT, "{name}");
        assert_eq!(
            error.message,
            format!("the bundle has no boot/{name}"),
            "{name}"
        );
    }
}

#[test]
fn a_listed_boot_file_that_is_not_carried_is_refused_with_the_boards_words() {
    // Each name listed in slot.json but not carried: the board's refusal at install.rs:683.
    for index in 0..5 {
        let full = bundle::boot_files(CMDLINE);
        let name = full[index].0;
        let carried: Vec<(&str, &[u8])> = full
            .iter()
            .copied()
            .filter(|(path, _)| *path != name)
            .collect();
        let error = verify(&bundle_listing(&full, &carried)).expect_err("refused");
        assert_eq!(error.code, code::INVALID_ARGUMENT, "{name}");
        assert_eq!(
            error.message,
            format!("the bundle has no boot file {name}"),
            "{name}"
        );
    }
}

#[test]
fn a_sixth_boot_entry_is_refused_by_name() {
    // Listed and carried: this board's own words until Lemnos I5i9 names the refusal.
    let mut sixth = bundle::boot_files(CMDLINE).to_vec();
    let extra: &[u8] = b"extra bytes";
    sixth.push(("overlays/extra.dtbo", extra));
    let error = verify(&bundle_listing(&sixth, &sixth)).expect_err("refused");
    assert_eq!(error.code, code::INVALID_ARGUMENT);
    assert!(
        error.message.starts_with(
            "boot.files lists overlays/extra.dtbo, which is not one of the five boot files"
        ),
        "{}",
        error.message
    );
}

#[test]
fn a_boot_member_the_manifest_does_not_list_is_refused_with_the_boards_words() {
    // Carried but not listed: install.rs:211, the board's words at the member's header.
    let boot = bundle::boot_files(CMDLINE);
    let mut carried = boot.to_vec();
    carried.push(("overlays/extra.dtbo", &b"extra bytes"[..]));
    let error = verify(&bundle_listing(&boot, &carried)).expect_err("refused");
    assert_eq!(
        error.message,
        "boot file overlays/extra.dtbo isn't in slot.json: a file the manifest does not list is refused"
    );
}
