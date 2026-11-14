//! The slot bundle's rules as Lemnos I5g2 and I5i4 state them, checked on the simulated board's
//! own functions: the member order and set, the record and its message, the command line, the
//! boot-time refusal and the restart refusal. Each refusal is pinned by name.

use atlas_sim_node::bundle::{self, BootRule, Verified};
use atlas_sim_node::slots::{NewSlot, Signed, Slots, State, Status, TrustedKey, code};
use ed25519_dalek::{Signer, SigningKey};

/// The complete command line the default board rule takes: its boot-source word is on it.
const CMDLINE: &[u8] = b"console=ttyS0 os.src.rpi-boot-mode-1=/soc@107c000000/mmc@fff000\n";

fn signer() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn held(key: &SigningKey) -> Vec<TrustedKey> {
    vec![TrustedKey {
        public: key.verifying_key().to_bytes(),
        owner: "Test Owner".into(),
    }]
}

/// The members of a signed slot, in the writer's order, as (name, bytes).
fn parts(id: &str, key: Option<&SigningKey>) -> Vec<(String, Vec<u8>)> {
    let rootfs = format!("rootfs of {id}\n").repeat(64).into_bytes();
    let boot: [(&str, &[u8]); 1] = [("cmdline.txt", CMDLINE)];
    let manifest = bundle::manifest(id, "2.0.0", &rootfs, &boot);
    let mut out = vec![("slot.json".to_string(), manifest.to_string().into_bytes())];
    if let Some(key) = key {
        out.push((
            "slot.sig".to_string(),
            bundle::sign(&manifest, key).to_vec(),
        ));
    }
    out.push(("rootfs.erofs".to_string(), rootfs));
    out.push(("boot/cmdline.txt".to_string(), CMDLINE.to_vec()));
    out
}

fn named(list: &[(String, Vec<u8>)], name: &str) -> Vec<u8> {
    list.iter()
        .find(|(n, _)| n == name)
        .map(|(_, bytes)| bytes.clone())
        .expect("member")
}

fn refused(parts: &[(String, Vec<u8>)], trusted: &[TrustedKey]) -> Status {
    bundle::verify(
        &bundle::tar_of(parts),
        trusted,
        "simulated",
        &BootRule::default(),
    )
    .expect_err("refused")
}

fn accepted(parts: &[(String, Vec<u8>)], trusted: &[TrustedKey]) -> Verified {
    bundle::verify(
        &bundle::tar_of(parts),
        trusted,
        "simulated",
        &BootRule::default(),
    )
    .expect("accepted")
}

#[test]
fn the_members_are_checked_in_order_and_the_order_is_named() {
    let key = signer();
    let list = parts("order", Some(&key));
    let json = named(&list, "slot.json");
    let sig = named(&list, "slot.sig");
    let rootfs = named(&list, "rootfs.erofs");
    let cmdline = named(&list, "boot/cmdline.txt");
    let trust = held(&key);

    let first = refused(
        &[
            ("rootfs.erofs".into(), rootfs.clone()),
            ("slot.json".into(), json.clone()),
        ],
        &trust,
    );
    assert_eq!(first.code, code::INVALID_ARGUMENT);
    assert!(
        first.message.contains("slot.json must be the first member"),
        "{}",
        first.message
    );

    let late_sig = refused(
        &[
            ("slot.json".into(), json.clone()),
            ("rootfs.erofs".into(), rootfs.clone()),
            ("slot.sig".into(), sig.clone()),
            ("boot/cmdline.txt".into(), cmdline.clone()),
        ],
        &trust,
    );
    assert_eq!(late_sig.code, code::INVALID_ARGUMENT);
    assert!(
        late_sig.message.contains("slot.sig is out of order"),
        "{}",
        late_sig.message
    );

    // Any order of the rootfs and the boot files is legal, after slot.json and slot.sig.
    let swapped = [
        ("slot.json".into(), json.clone()),
        ("slot.sig".into(), sig.clone()),
        ("boot/cmdline.txt".into(), cmdline.clone()),
        ("rootfs.erofs".into(), rootfs.clone()),
    ];
    assert_eq!(
        accepted(&swapped, &trust).signed,
        Signed::Trusted {
            owner: "Test Owner".into()
        }
    );

    // An unsigned bundle is legal: no slot.sig at all.
    let unsigned = [
        ("slot.json".into(), json.clone()),
        ("rootfs.erofs".into(), rootfs.clone()),
        ("boot/cmdline.txt".into(), cmdline.clone()),
    ];
    assert_eq!(accepted(&unsigned, &trust).signed, Signed::Unsigned);
}

#[test]
fn a_member_outside_the_set_is_refused_at_its_header_by_name() {
    let key = signer();
    let mut list = parts("unlisted", Some(&key));
    list.push(("slot.key".into(), b"0".repeat(64)));
    let error = refused(&list, &held(&key));
    assert_eq!(error.code, code::INVALID_ARGUMENT);
    assert!(
        error
            .message
            .contains("slot.key is not part of a slot bundle"),
        "{}",
        error.message
    );
}

#[test]
fn a_boot_file_slot_json_does_not_list_is_refused_by_name() {
    let key = signer();
    let mut list = parts("extra", Some(&key));
    list.push(("boot/extra.txt".into(), b"x".to_vec()));
    let error = refused(&list, &held(&key));
    assert!(
        error.message.contains("boot/extra.txt is not in slot.json"),
        "{}",
        error.message
    );
}

#[test]
fn a_listed_member_that_is_missing_is_refused_by_name() {
    let key = signer();
    let mut list = parts("missing", Some(&key));
    list.retain(|(name, _)| name != "boot/cmdline.txt");
    let error = refused(&list, &held(&key));
    assert_eq!(error.code, code::INVALID_ARGUMENT);
    assert!(
        error
            .message
            .contains("boot/cmdline.txt is listed in slot.json but missing"),
        "{}",
        error.message
    );
}

#[test]
fn a_signature_over_the_bare_manifest_bytes_by_a_held_key_is_refused() {
    let key = signer();
    let list = parts("bare", Some(&key));
    let json = named(&list, "slot.json");
    let mut record = named(&list, "slot.sig");
    // The held key's record, with its signature over the bare bytes: the old form.
    let bare = key.sign(&json).to_bytes();
    record[44..108].copy_from_slice(&bare);
    let mut changed = list.clone();
    changed[1].1 = record;
    let error = refused(&changed, &held(&key));
    assert_eq!(error.code, code::NOT_PERMITTED, "{}", error.message);
    assert!(
        error.message.contains("does not verify"),
        "{}",
        error.message
    );
}

#[test]
fn malformed_records_are_invalid_argument_whatever_the_key_list() {
    let key = signer();
    let list = parts("malformed", Some(&key));
    let good = named(&list, "slot.sig");
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("the bare 64-byte form", good[..64].to_vec()),
        ("107 bytes", good[..107].to_vec()),
        ("a bad magic", {
            let mut bytes = good.clone();
            bytes[0] ^= 1;
            bytes
        }),
        ("algorithm 2", {
            let mut bytes = good.clone();
            bytes[8] = 2;
            bytes
        }),
        ("padding that is not zero", {
            let mut bytes = good.clone();
            bytes[11] = 1;
            bytes
        }),
    ];
    for (what, record) in cases {
        let mut changed = list.clone();
        changed[1].1 = record;
        for trust in [Vec::new(), held(&key)] {
            let error = refused(&changed, &trust);
            assert_eq!(
                error.code,
                code::INVALID_ARGUMENT,
                "{what}: {}",
                error.message
            );
        }
    }
}

#[test]
fn an_unknown_key_is_not_checked_and_not_refused() {
    let key = signer();
    let stranger = SigningKey::from_bytes(&[9; 32]);
    let list = parts("stranger", Some(&stranger));
    let verified = accepted(&list, &held(&key));
    assert_eq!(verified.signed, Signed::Untrusted);
    let verified = accepted(&list, &[]);
    assert_eq!(verified.signed, Signed::Untrusted);
}

#[test]
fn the_cmdline_must_carry_the_word_for_each_boot_source() {
    let key = signer();
    let list = parts("cmdline", Some(&key));
    let raw = bundle::tar_of(&list);
    // The default rule: the word is there, and a line without it is refused by name.
    assert!(bundle::verify(&raw, &held(&key), "simulated", &BootRule::default()).is_ok());
    // A second source the line does not carry: refused, naming the word.
    let rule = BootRule {
        boot_args: "os".into(),
        sources: vec![
            (
                "rpi-boot-mode-1".into(),
                "/soc@107c000000/mmc@fff000".into(),
            ),
            ("mmc1".into(), "/soc/mmc@fe2e0000".into()),
        ],
    };
    let error = bundle::verify(&raw, &held(&key), "simulated", &rule).expect_err("refused");
    assert_eq!(error.code, code::INVALID_ARGUMENT);
    assert!(
        error.message.contains("os.src.mmc1=/soc/mmc@fe2e0000"),
        "{}",
        error.message
    );
}

fn new_slot(id: &str, boot: &[(&str, &str)]) -> NewSlot {
    NewSlot {
        id: id.into(),
        product: "sim".into(),
        board: "simulated".into(),
        version: "2.0.0".into(),
        variant: String::new(),
        kernel: String::new(),
        display_name: id.into(),
        size: 4096,
        verity: true,
        signed: Signed::Unsigned,
        boot: boot
            .iter()
            .map(|(path, digest)| ((*path).to_string(), (*digest).to_string()))
            .collect(),
    }
}

/// A board whose slot `B` is the default, with `slot-1` as its previous slot.
fn board_with_default_b() -> Slots {
    let mut slots = Slots::new();
    slots
        .install_done(
            new_slot("B", &[("cmdline.txt", "c1"), ("kernel8.img", "k1")]),
            false,
            true,
        )
        .expect("installs");
    slots.trial("B", false, true, false).expect("tried");
    slots.confirm("test").expect("confirmed");
    assert_eq!(slots.default.as_deref(), Some("B"));
    assert_eq!(slots.active.as_deref(), Some("B"));
    slots
}

#[test]
fn a_boot_whose_kernel_differs_from_the_manifest_does_not_run_and_the_previous_slot_does() {
    let mut slots = board_with_default_b();
    slots
        .set_boot_file("B", "kernel8.img", Some("tampered"))
        .expect("set");
    // The boot path (a power-on, not a restart call): B is refused, slot-1 boots.
    slots.reboot();
    assert_eq!(slots.active.as_deref(), Some("slot-1"));
    assert!(
        slots
            .events
            .iter()
            .any(|event| event.contains("B refused at boot") && event.contains("boot/kernel8.img")),
        "{:?}",
        slots.events
    );
}

#[test]
fn a_boot_whose_cmdline_differs_and_no_slot_matches_says_why_and_runs_nothing() {
    let mut slots = board_with_default_b();
    slots
        .set_boot_file("B", "cmdline.txt", Some("tampered"))
        .expect("set");
    slots
        .set_boot_file("slot-1", "cmdline.txt", Some("stray"))
        .expect("set");
    slots.reboot();
    assert_eq!(slots.active, None);
    assert_eq!(slots.state, State::Error);
    assert!(slots.error.contains("boot/cmdline.txt"), "{}", slots.error);
}

#[test]
fn a_restart_is_refused_by_name_when_the_default_would_not_boot() {
    let mut slots = board_with_default_b();
    slots
        .set_boot_file("B", "kernel8.img", Some("tampered"))
        .expect("set");
    let refusal = slots.check_restart().expect_err("refused");
    assert_eq!(refusal.code, code::NOT_PERMITTED);
    assert!(
        refusal.message.contains("boot/kernel8.img"),
        "{}",
        refusal.message
    );
    // Nothing restarted: the board still runs B.
    assert_eq!(slots.active.as_deref(), Some("B"));
}

#[test]
fn a_trial_into_a_slot_whose_files_differ_is_refused_at_the_call() {
    let mut slots = board_with_default_b();
    slots
        .install_done(new_slot("D", &[("kernel8.img", "kd")]), false, true)
        .expect("installs");
    slots
        .set_boot_file("D", "kernel8.img", Some("tampered"))
        .expect("set");
    let refusal = slots.trial("D", false, true, false).expect_err("refused");
    assert_eq!(refusal.code, code::NOT_PERMITTED);
    assert!(
        refusal.message.contains("boot/kernel8.img"),
        "{}",
        refusal.message
    );
    assert!(slots.trial.is_none(), "no trial was set");
    assert_eq!(slots.active.as_deref(), Some("B"));
}

#[test]
fn a_trial_whose_files_differ_at_boot_falls_back_at_once() {
    let mut slots = board_with_default_b();
    slots
        .install_done(new_slot("C", &[("kernel8.img", "kc")]), false, true)
        .expect("installs");
    slots.trial("C", false, true, false).expect("tried");
    // The power goes during the trial's boot: the trial is pending, not reached.
    slots.power_cut();
    slots
        .set_boot_file("C", "kernel8.img", Some("tampered"))
        .expect("set");
    slots.reboot();
    let resolved = slots.resolved.clone().expect("resolved");
    assert!(!resolved.confirmed);
    assert!(
        resolved.reason.contains("does not boot"),
        "{}",
        resolved.reason
    );
    assert!(
        resolved.reason.contains("boot/kernel8.img"),
        "{}",
        resolved.reason
    );
    assert_eq!(slots.active.as_deref(), Some("B"));
}

#[test]
fn a_verified_bundle_reports_its_boot_files_for_the_install_to_keep() {
    let key = signer();
    let list = parts("boots", Some(&key));
    let verified = accepted(&list, &held(&key));
    assert!(verified.boot.contains_key("cmdline.txt"));
}
