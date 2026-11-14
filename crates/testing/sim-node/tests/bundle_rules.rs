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

fn events_with(slots: &Slots, prefix: &str) -> usize {
    slots
        .events
        .iter()
        .filter(|event| event.starts_with(prefix))
        .count()
}

#[test]
fn a_trial_whose_boot_files_do_not_match_fails_by_the_failure_path() {
    let mut slots = board_with_default_b();
    slots
        .install_done(
            new_slot("C", &[("cmdline.txt", "c1"), ("kernel8.img", "kc")]),
            false,
            true,
        )
        .expect("installs");
    slots.trial("C", false, true, false).expect("tried");
    // The power goes during the trial's boot: the trial is pending, not reached.
    slots.power_cut();
    slots
        .set_boot_file("C", "kernel8.img", Some("tampered"))
        .expect("set");
    slots.reboot();
    let trial = slots.get("C").expect("C");
    assert!(trial.failed, "the trial is marked failed");
    assert_eq!(trial.failures, 1, "one failure counted");
    assert_eq!(
        trial.fail_reason,
        "kernel8.img doesn't match its manifest hash"
    );
    let resolved = slots.resolved.clone().expect("resolved");
    assert!(!resolved.confirmed);
    assert_eq!(
        resolved.reason,
        "kernel8.img doesn't match its manifest hash"
    );
    assert_eq!(
        slots.active.as_deref(),
        Some("B"),
        "the board falls back to the default"
    );
}

#[test]
fn a_default_that_does_not_match_is_swapped_with_its_previous_at_boot_with_no_count() {
    let mut slots = board_with_default_b();
    slots
        .set_boot_file("B", "kernel8.img", Some("tampered"))
        .expect("set");
    slots.reboot();
    assert_eq!(
        slots.active.as_deref(),
        Some("slot-1"),
        "the previous boots"
    );
    assert_eq!(slots.default.as_deref(), Some("slot-1"));
    assert_eq!(slots.previous.as_deref(), Some("B"));
    let old = slots.get("B").expect("B");
    assert!(old.failed);
    assert_eq!(old.failures, 0, "no count: the immediate previous");
    assert_eq!(
        old.fail_reason,
        "its boot files don't match its manifest: kernel8.img doesn't match its manifest hash"
    );
    assert!(
        slots.events.contains(&"slot B's boot files don't match its manifest: kernel8.img doesn't match its manifest hash; slot-1 is the default now, restarting into it".to_string()),
        "{:?}",
        slots.events
    );
}

/// A board whose default is `C` and previous `B`, both with boot files.
fn board_with_default_c_and_previous_b() -> Slots {
    let mut slots = board_with_default_b();
    slots
        .install_done(
            new_slot("C", &[("cmdline.txt", "c1"), ("kernel8.img", "kc")]),
            false,
            true,
        )
        .expect("installs");
    slots.trial("C", false, true, false).expect("tried");
    slots.confirm("test").expect("confirmed");
    assert_eq!(slots.default.as_deref(), Some("C"));
    assert_eq!(slots.previous.as_deref(), Some("B"));
    slots
}

#[test]
fn no_slot_can_start_when_the_default_and_its_previous_both_do_not_match() {
    let mut slots = board_with_default_c_and_previous_b();
    slots
        .set_boot_file("C", "kernel8.img", Some("tampered"))
        .expect("set");
    slots
        .set_boot_file("B", "cmdline.txt", Some("tampered"))
        .expect("set");
    slots.reboot();
    assert_eq!(slots.active, None);
    assert_eq!(slots.state, State::Error);
    assert_eq!(
        slots.error,
        "no slot can start: kernel8.img doesn't match its manifest hash; its previous B doesn't match its manifest either: cmdline.txt doesn't match its manifest hash"
    );
    // No pointer moved.
    assert_eq!(slots.default.as_deref(), Some("C"));
    assert_eq!(slots.previous.as_deref(), Some("B"));
    // The board restarts again after its rescue wait, on the simulated clock.
    let before = events_with(&slots, "no slot can start");
    slots.advance_clock(slots.rescue_wait_s - 1);
    assert_eq!(events_with(&slots, "no slot can start"), before, "not yet");
    slots.advance_clock(1);
    assert!(
        events_with(&slots, "no slot can start") > before,
        "the board boots again"
    );
}

#[test]
fn a_default_with_no_valid_previous_fails_by_the_failure_path_and_nothing_starts() {
    let mut slots = board_with_default_b();
    slots.previous = None;
    slots
        .set_boot_file("B", "kernel8.img", Some("tampered"))
        .expect("set");
    slots.reboot();
    assert_eq!(slots.active, None);
    assert_eq!(
        slots.error,
        "no slot can start: kernel8.img doesn't match its manifest hash"
    );
    let failed = slots.get("B").expect("B");
    assert!(failed.failed);
    assert_eq!(failed.failures, 1);
    assert_eq!(slots.default.as_deref(), Some("B"), "no pointer moved");
}

#[test]
fn a_restart_repoints_when_the_default_does_not_match_and_its_previous_does() {
    let mut slots = board_with_default_b();
    slots
        .set_boot_file("B", "kernel8.img", Some("tampered"))
        .expect("set");
    let note = slots.check_restart().expect("the restart proceeds");
    assert_eq!(
        note.as_deref(),
        Some(
            "B doesn't match its manifest (kernel8.img doesn't match its manifest hash): slot-1 is the default now"
        )
    );
    slots.reboot();
    assert_eq!(slots.active.as_deref(), Some("slot-1"));
}

#[test]
fn a_restart_is_refused_when_neither_the_default_nor_its_previous_matches() {
    let mut slots = board_with_default_c_and_previous_b();
    slots
        .set_boot_file("C", "kernel8.img", Some("tampered"))
        .expect("set");
    slots
        .set_boot_file("B", "cmdline.txt", Some("tampered"))
        .expect("set");
    let refusal = slots.check_restart().expect_err("refused");
    assert_eq!(refusal.code, code::NOT_PERMITTED);
    assert_eq!(
        refusal.message,
        "C doesn't match its manifest (kernel8.img doesn't match its manifest hash), and its previous \"B\" doesn't either: the restart is refused"
    );
    // The board stays up on the default.
    assert_eq!(slots.active.as_deref(), Some("C"));
    assert_eq!(slots.default.as_deref(), Some("C"));
}

#[test]
fn the_periodic_check_only_reports_when_neither_matches() {
    let mut slots = board_with_default_c_and_previous_b();
    slots
        .set_boot_file("C", "kernel8.img", Some("tampered"))
        .expect("set");
    slots
        .set_boot_file("B", "cmdline.txt", Some("tampered"))
        .expect("set");
    slots.advance_clock(atlas_sim_node::slots::boot::PERIODIC_CHECK_S);
    let report = "C doesn't match its manifest (kernel8.img doesn't match its manifest hash), and its previous \"B\" doesn't either";
    assert!(
        slots.events.iter().any(|event| event == report),
        "{:?}",
        slots.events
    );
    assert!(
        !slots
            .events
            .iter()
            .any(|event| event.ends_with("the restart is refused"))
    );
    assert_eq!(slots.default.as_deref(), Some("C"), "no pointer moved");
}

#[test]
fn the_mismatch_list_names_each_file_missing_or_changed_joined_by_semicolons() {
    let mut slots = board_with_default_b();
    slots
        .set_boot_file("B", "kernel8.img", Some("tampered"))
        .expect("set");
    slots.set_boot_file("B", "cmdline.txt", None).expect("set");
    assert_eq!(
        slots.why("B").as_deref(),
        Some(
            "cmdline.txt is missing from the boot partition; kernel8.img doesn't match its manifest hash"
        )
    );
}

#[test]
fn a_verified_bundle_reports_its_boot_files_for_the_install_to_keep() {
    let key = signer();
    let list = parts("boots", Some(&key));
    let verified = accepted(&list, &held(&key));
    assert!(verified.boot.contains_key("cmdline.txt"));
}
