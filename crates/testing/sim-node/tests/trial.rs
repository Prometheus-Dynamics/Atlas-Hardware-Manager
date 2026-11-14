//! The trial lifecycle (I5f), the list reply (`list`, M8a), the signature outcomes (I5g), and the
//! install and removal refusals, on the simulated node. Time is the simulated clock: each test
//! moves it with `slot_clock_advance`, so nothing waits on the wall clock.
#![cfg(target_os = "linux")]

use std::io::Cursor;

use atlas_sim_node::bundle;
use atlas_sim_node::names;
use atlas_sim_node::slots::NewSlot;
use atlas_sim_node::slots::Signed;
use atlas_sim_node::slots::trial::Config;
use atlas_sim_node::{DEADLINE, SimNode, call};
use ed25519_dalek::{Signer, SigningKey};
use orion::Orion;
use orion::actions::{ActionCall, Finished, Outcome};
use serde_json::Value;

/// A node for one test, in its own directory.
fn start(name: &str) -> SimNode {
    // Under the system temp dir and short: the node's Unix socket path must fit SUN_LEN, and a
    // long build path (CI's workspace) would not.
    let dir = std::env::temp_dir().join(format!("atl-trial-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    SimNode::start(&dir).expect("the simulated node starts")
}

/// The command line the default board rule takes: its boot-source word is on it.
const CMDLINE: &[u8] =
    b"console=ttyS0 root=/dev/dm-0 os.src.rpi-boot-mode-1=/soc@107c000000/mmc@fff000\n";

/// A bundle for slot `id`, signed by `key` when given (its `slot.sig` record).
fn bundle_for(id: &str, version: &str, key: Option<&SigningKey>) -> Vec<u8> {
    let rootfs = format!("rootfs of {id} at {version}\n")
        .repeat(900)
        .into_bytes();
    let boot = bundle::boot_files(CMDLINE);
    let manifest = bundle::manifest(id, version, &rootfs, &boot);
    let record = key.map(|key| bundle::sign(&manifest, key));
    bundle::build(
        &manifest,
        record.as_ref().map(|record| &record[..]),
        &rootfs,
        &boot,
    )
}

const BUNDLE: &str = "bundle.tar";

fn push(orion: &Orion, name: &str, bytes: &[u8]) {
    let destination = format!("{}{name}", names::INCOMING);
    orion
        .files()
        .push(&destination, &mut Cursor::new(bytes), |_, _| {})
        .expect("the bundle is pushed");
}

/// Accepts, pushes and installs a bundle, and says how the install ended.
fn install(orion: &Orion, bundle: &[u8], pin: bool, keep_old: bool) -> Finished {
    let accepted = call(
        orion,
        names::SLOTS_ACCEPT,
        bundle.len().to_string().as_bytes(),
    )
    .expect("accept answers");
    if accepted.outcome != Outcome::Succeeded {
        return accepted;
    }
    push(orion, BUNDLE, bundle);
    let args = format!("{BUNDLE} {pin} {keep_old}");
    orion
        .run(&ActionCall {
            name: names::SLOTS_INSTALL_FILE,
            target: atlas_sim_node::node::target(),
            args: args.as_bytes(),
            deadline_ms: 0,
            detached: false,
        })
        .expect("the install starts")
        .finish(DEADLINE)
        .expect("the install ends")
}

fn ok(orion: &Orion, name: &str, args: &[u8]) -> Finished {
    let finished = call(orion, name, args).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(
        finished.outcome,
        Outcome::Succeeded,
        "{name} {args:?}: {}",
        finished.detail
    );
    finished
}

fn refused(orion: &Orion, name: &str, args: &[u8]) -> Finished {
    let finished = call(orion, name, args).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_ne!(
        finished.outcome,
        Outcome::Succeeded,
        "{name} {args:?} should be refused"
    );
    finished
}

fn list(orion: &Orion) -> Value {
    let finished = call(orion, names::SLOTS_LIST, b"").expect("list answers");
    assert_eq!(finished.outcome, Outcome::Succeeded, "{}", finished.detail);
    serde_json::from_slice(&finished.output).expect("list is JSON")
}

fn slot<'a>(listing: &'a Value, id: &str) -> &'a Value {
    listing["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|slot| slot["id"] == id)
        .unwrap_or_else(|| panic!("no slot {id} in {listing}"))
}

fn slot_count(listing: &Value) -> usize {
    listing["slots"].as_array().expect("slots").len()
}

/// A node with `slot-2` installed (unsigned, not tried), and a session to it.
fn node_with_slot_2(name: &str) -> (SimNode, Orion) {
    let node = start(name);
    let orion = node.connect(name).expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    (node, orion)
}

#[test]
fn list_has_the_i5f_fields_null_by_default_and_an_outcome_per_slot() {
    let node = start("list-shape");
    let orion = node.connect("list-shape").expect("a session");
    let listing = list(&orion);
    assert!(listing["trial"].is_null(), "{listing}");
    assert!(listing["resolved"].is_null(), "{listing}");
    // Every field of the earlier shape is still there.
    for key in [
        "slot_active",
        "slot_default",
        "slot_previous",
        "slot_trial",
        "slot_staged",
        "state",
        "error",
        "free_bytes",
        "reserve_bytes",
        "keep",
        "slots",
    ] {
        assert!(listing.get(key).is_some(), "{key} is in {listing}");
    }
    // The default slot is unsigned: its outcome is not checked, and its signed label is kept.
    let first = slot(&listing, "slot-1");
    assert_eq!(first["signature"], "not_checked_unsigned");
    assert_eq!(first["signed"], "unsigned");
    assert_eq!(first["signer"], "");
}

#[test]
fn a_list_over_60_kib_is_refused_by_name_with_no_output() {
    let node = start("list-limit");
    let orion = node.connect("list-limit").expect("a session");
    // Each slot is about 480 bytes of JSON, so 300 of them are well over 60 KiB. They go in
    // straight to the model (kept, so housekeeping does not remove them).
    {
        let mut model = node.model();
        for index in 0..300 {
            model
                .slots
                .install_done(
                    NewSlot {
                        boot: Default::default(),
                        id: format!("bulk-{index:03}"),
                        product: "sim".into(),
                        board: "simulated".into(),
                        version: "1.0.0".into(),
                        variant: String::new(),
                        kernel: String::new(),
                        display_name: format!("Bulk {index}"),
                        size: 4096,
                        verity: true,
                        signed: Signed::Unsigned,
                    },
                    false,
                    true,
                )
                .expect("the slot is installed");
        }
    }
    let finished = call(&orion, names::SLOTS_LIST, b"").expect("list answers");
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert_eq!(
        finished.detail,
        "-22: system.slots.list: the reply is over 60 KiB; refused by name, nothing partial is sent"
    );
    assert!(finished.output.is_empty(), "no partial output is sent");
}

#[test]
fn a_healthy_trial_confirms_itself_after_the_uptime_on_the_clock() {
    let (node, orion) = node_with_slot_2("trial-healthy");
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    let trying = list(&orion);
    assert_eq!(trying["slot_active"], "slot-2");
    assert_eq!(trying["trial"]["slot"], "slot-2");
    assert_eq!(trying["trial"]["attempts_left"], 3);
    assert_eq!(trying["trial"]["last"], "daemon-reached-not-yet-confirmed");
    assert_eq!(trying["trial"]["attempts"][0]["n"], 1);
    assert!(trying["resolved"].is_null());

    // Not yet: the daemon has run 29 s of the 30 it must.
    node.slot_clock_advance(29);
    assert_eq!(list(&orion)["trial"]["slot"], "slot-2");

    node.slot_clock_advance(1);
    let confirmed = list(&orion);
    assert!(confirmed["trial"].is_null(), "{confirmed}");
    assert_eq!(confirmed["slot_default"], "slot-2");
    assert_eq!(confirmed["resolved"]["slot"], "slot-2");
    assert_eq!(confirmed["resolved"]["outcome"], "confirmed");
    assert_eq!(confirmed["resolved"]["by"], "auto");
    assert!(confirmed["resolved"]["caller"].is_null());
    assert!(
        confirmed["resolved"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("healthy")),
        "{confirmed}"
    );
}

#[test]
fn an_unhealthy_trial_falls_back_at_the_deadline_and_keeps_the_reason() {
    let (node, orion) = node_with_slot_2("trial-deadline");
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    node.slot_set_healthy(false);

    // Past the uptime, but not healthy: nothing confirms, and nothing restarts yet.
    node.slot_clock_advance(30);
    assert_eq!(list(&orion)["trial"]["slot"], "slot-2");
    node.slot_clock_advance(569);
    assert_eq!(
        list(&orion)["trial"]["slot"],
        "slot-2",
        "599 s is before the deadline"
    );

    node.slot_clock_advance(1);
    let fell_back = list(&orion);
    assert!(fell_back["trial"].is_null(), "{fell_back}");
    assert_eq!(fell_back["slot_active"], "slot-1", "back on the default");
    assert_eq!(fell_back["state"], "rolled-back");
    assert_eq!(slot(&fell_back, "slot-2")["failed"], true);
    assert_eq!(fell_back["resolved"]["outcome"], "fell-back");
    assert_eq!(fell_back["resolved"]["by"], "auto");
    assert_eq!(
        fell_back["resolved"]["reason"],
        "a health condition is not met: the board is not healthy"
    );
}

#[test]
fn a_trial_with_auto_confirm_off_is_not_restarted_at_its_deadline() {
    let (node, orion) = node_with_slot_2("trial-manual");
    node.slot_config(|config: &mut Config| config.auto_confirm = false);
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    node.slot_set_healthy(false);
    node.slot_clock_advance(10_000);
    let still = list(&orion);
    assert_eq!(still["trial"]["slot"], "slot-2", "{still}");
    assert_eq!(still["slot_active"], "slot-2");
    assert!(still["resolved"].is_null());
}

#[test]
fn three_boots_that_never_reach_the_daemon_fall_back_and_name_the_count() {
    let (node, orion) = node_with_slot_2("trial-boots");
    node.slot_boots_without_daemon(3);
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");

    let first = list(&orion);
    assert_eq!(first["trial"]["slot"], "slot-2", "still pending: {first}");
    assert_eq!(first["trial"]["attempts_left"], 2);
    assert_eq!(first["trial"]["last"], "power-lost-before-daemon");
    assert_eq!(
        first["slot_active"], "slot-1",
        "the board is on the default"
    );

    ok(&orion, names::REBOOT, b"");
    let second = list(&orion);
    assert_eq!(second["trial"]["attempts_left"], 1);
    assert_eq!(
        second["trial"]["attempts"]
            .as_array()
            .expect("attempts")
            .len(),
        2
    );

    ok(&orion, names::REBOOT, b"");
    let done = list(&orion);
    assert!(done["trial"].is_null(), "{done}");
    assert_eq!(done["slot_active"], "slot-1");
    assert_eq!(done["resolved"]["outcome"], "fell-back");
    assert_eq!(done["resolved"]["by"], "auto");
    assert_eq!(
        done["resolved"]["reason"],
        "the trial slot did not reach a running daemon in 3 boots"
    );
    assert_eq!(slot(&done, "slot-2")["failed"], true);
}

#[test]
fn a_boot_that_reached_the_daemon_and_was_cut_short_costs_no_attempt() {
    let (node, orion) = node_with_slot_2("trial-cut");
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    node.slot_power_cut();
    let cut = list(&orion);
    assert_eq!(
        cut["trial"]["slot"], "slot-2",
        "a cut trial stays pending: {cut}"
    );
    assert_eq!(cut["trial"]["attempts_left"], 3, "no attempt spent");
    assert_eq!(cut["slot_active"], "slot-1");

    // The next restart boots the trial again, and it reaches the daemon.
    ok(&orion, names::REBOOT, b"");
    let again = list(&orion);
    assert_eq!(again["slot_active"], "slot-2");
    assert_eq!(again["trial"]["attempts_left"], 3);
    assert_eq!(
        again["trial"]["attempts"]
            .as_array()
            .expect("attempts")
            .len(),
        2
    );

    // Two boots that never reach the daemon, then the counter reads one left.
    node.slot_power_cut();
    node.slot_boots_without_daemon(2);
    ok(&orion, names::REBOOT, b"");
    ok(&orion, names::REBOOT, b"");
    let counted = list(&orion);
    assert_eq!(counted["trial"]["attempts_left"], 1, "{counted}");
    assert_eq!(counted["trial"]["slot"], "slot-2");
}

#[test]
fn an_explicit_confirm_of_the_default_clears_a_pending_trial() {
    let (node, orion) = node_with_slot_2("trial-clear");
    node.slot_boots_without_daemon(1);
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    let pending = list(&orion);
    assert_eq!(pending["trial"]["slot"], "slot-2");
    assert_eq!(pending["slot_active"], "slot-1", "the default is running");

    ok(&orion, names::SLOTS_CONFIRM, b"");
    let cleared = list(&orion);
    assert!(cleared["trial"].is_null(), "{cleared}");
    assert_eq!(cleared["slot_default"], "slot-1", "the default is kept");
    assert_eq!(cleared["resolved"]["slot"], "slot-2");
    assert_eq!(cleared["resolved"]["outcome"], "fell-back");
    assert_eq!(cleared["resolved"]["by"], "call");
    assert!(
        cleared["resolved"]["caller"]
            .as_str()
            .is_some_and(|caller| !caller.is_empty()),
        "the caller is named: {cleared}"
    );
}

#[test]
fn a_failed_record_write_sets_record_error_and_writes_the_trial_reason() {
    let (node, orion) = node_with_slot_2("trial-record");
    node.slot_fail_record_writes(true);
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    let carrying = list(&orion);
    assert_eq!(
        carrying["trial"]["record_error"], "the state directory refused the write",
        "{carrying}"
    );
    assert!(
        node.model().slots.trial_reason.is_none(),
        "nothing fell back yet"
    );

    // The deadline passes with the record unwritable: the trial falls back, and its reason goes to
    // the boot partition's trial-reason line.
    node.slot_clock_advance(600);
    let fell_back = list(&orion);
    assert!(fell_back["trial"].is_null());
    let reason = fell_back["resolved"]["reason"].as_str().expect("a reason");
    assert!(
        reason.contains("the state could not be written"),
        "{reason}"
    );
    assert_eq!(
        node.model().slots.trial_reason.as_deref(),
        Some(reason),
        "trial-reason holds the reason"
    );

    // The writes work again. The reason stays on the boot partition until a resolution clears it.
    node.slot_fail_record_writes(false);
    assert!(node.model().slots.trial_reason.is_some());
}

#[test]
fn signature_outcomes_are_verified_unsigned_and_unknown_key_and_installed() {
    let node = start("signature-ok");
    let orion = node.connect("signature-ok").expect("a session");
    let trusted = SigningKey::from_bytes(&[7; 32]);
    let stranger = SigningKey::from_bytes(&[9; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_sim_node::slots::TrustedKey {
            public: trusted.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });

    let verified = install(
        &orion,
        &bundle_for("verified", "2.0.0", Some(&trusted)),
        false,
        true,
    );
    assert_eq!(verified.outcome, Outcome::Succeeded, "{}", verified.detail);
    let unsigned = install(&orion, &bundle_for("unsigned", "3.0.0", None), false, true);
    assert_eq!(unsigned.outcome, Outcome::Succeeded, "{}", unsigned.detail);
    // Signed by a key the board does not hold: accepted, not checked.
    let unknown = install(
        &orion,
        &bundle_for("unknown", "4.0.0", Some(&stranger)),
        false,
        true,
    );
    assert_eq!(unknown.outcome, Outcome::Succeeded, "{}", unknown.detail);

    let listing = list(&orion);
    assert_eq!(slot(&listing, "verified")["signature"], "verified");
    assert_eq!(slot(&listing, "verified")["signed"], "trusted");
    assert_eq!(slot(&listing, "verified")["signer"], "Test Owner");
    assert_eq!(
        slot(&listing, "unsigned")["signature"],
        "not_checked_unsigned"
    );
    assert_eq!(
        slot(&listing, "unknown")["signature"],
        "not_checked_unknown_key"
    );
    assert_eq!(slot(&listing, "unknown")["signed"], "untrusted");
    assert_eq!(slot(&listing, "unknown")["signer"], "");
    assert_eq!(
        slot(&listing, "unknown")["signature"],
        "not_checked_unknown_key"
    );
}

#[test]
fn a_bad_signature_from_a_known_key_and_a_malformed_record_are_refused_and_leave_nothing() {
    let node = start("signature-refused");
    let orion = node.connect("signature-refused").expect("a session");
    let trusted = SigningKey::from_bytes(&[7; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_sim_node::slots::TrustedKey {
            public: trusted.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let rootfs = b"rootfs of bad-sig\n".repeat(64);
    let boot = bundle::boot_files(CMDLINE);
    let manifest = bundle::manifest("bad-sig", "2.0.0", &rootfs, &boot);
    let record = bundle::sign(&manifest, &trusted);

    // A record by the trusted key whose signature does not verify (one byte changed).
    let mut forged_record = record;
    forged_record[60] ^= 0xff;
    let forged = bundle::build(&manifest, Some(&forged_record[..]), &rootfs, &boot);
    let refusal = install(&orion, &forged, false, false);
    assert_eq!(refusal.outcome, Outcome::Rejected, "{}", refusal.detail);
    assert!(refusal.detail.starts_with("-1:"), "{}", refusal.detail);
    assert!(
        refusal.detail.contains("does not verify"),
        "{}",
        refusal.detail
    );
    assert_eq!(slot_count(&list(&orion)), 1, "nothing installed");

    // The old bare 64-byte signature form, by the trusted key, is malformed.
    let short = bundle::build(&manifest, Some(&record[..64]), &rootfs, &boot);
    let finished = install(&orion, &short, false, false);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
    assert!(finished.detail.contains("108-byte"), "{}", finished.detail);

    // A record whose magic is wrong is malformed.
    let mut magic = record;
    magic[0] ^= 1;
    let bad_magic = bundle::build(&manifest, Some(&magic[..]), &rootfs, &boot);
    let finished = install(&orion, &bad_magic, false, false);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
    assert!(finished.detail.contains("LMNSSIG1"), "{}", finished.detail);

    // A signature by the trusted key over the BARE slot.json bytes does not verify.
    let bare = trusted.sign(manifest.to_string().as_bytes()).to_bytes();
    let mut bare_record = record;
    bare_record[44..108].copy_from_slice(&bare);
    let bare_bundle = bundle::build(&manifest, Some(&bare_record[..]), &rootfs, &boot);
    let finished = install(&orion, &bare_bundle, false, false);
    assert!(finished.detail.starts_with("-1:"), "{}", finished.detail);

    assert_eq!(
        slot_count(&list(&orion)),
        1,
        "nothing installed by any of them"
    );
}

#[test]
fn security_on_refuses_to_make_an_unverified_slot_the_default() {
    let (node, orion) = node_with_slot_2("security-default");
    node.model().slots.board_security = true;
    node.model()
        .slots
        .trusted
        .push(atlas_sim_node::slots::TrustedKey {
            public: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let finished = refused(&orion, names::SLOTS_SET_DEFAULT, b"slot-2 false");
    assert!(finished.detail.starts_with("-1:"), "{}", finished.detail);
    assert!(
        finished.detail.contains("not signed by a trusted key"),
        "{}",
        finished.detail
    );
    let finished = refused(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    assert!(finished.detail.starts_with("-1:"), "{}", finished.detail);
}

#[test]
fn a_slot_for_another_board_is_minus_95_at_set_default_and_trial_and_names_both_boards() {
    let (node, orion) = node_with_slot_2("board-95");
    node.model()
        .slots
        .entries
        .iter_mut()
        .find(|entry| entry.id == "slot-2")
        .expect("slot-2")
        .board = "another-board".into();
    let default = refused(&orion, names::SLOTS_SET_DEFAULT, b"slot-2 false");
    assert!(default.detail.starts_with("-95:"), "{}", default.detail);
    assert!(
        default
            .detail
            .contains("this slot is built for board another-board, and this is board simulated"),
        "{}",
        default.detail
    );
    let trial = refused(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    assert!(trial.detail.starts_with("-95:"), "{}", trial.detail);
    assert_eq!(list(&orion)["slot_active"], "slot-1", "nothing changed");
}

#[test]
fn a_removal_refusal_names_its_reason() {
    let (node, orion) = node_with_slot_2("removal");
    let bad = refused(&orion, names::SLOTS_REMOVE, b"a/b");
    assert!(bad.detail.starts_with("-22:"), "{}", bad.detail);
    let missing = refused(&orion, names::SLOTS_REMOVE, b"nope");
    assert!(missing.detail.starts_with("-2:"), "{}", missing.detail);
    let running = refused(&orion, names::SLOTS_REMOVE, b"slot-1");
    assert!(running.detail.starts_with("-16:"), "{}", running.detail);

    // A pending trial's slot is busy too.
    ok(&orion, names::SLOTS_TRIAL, b"slot-2 false false false");
    // The boot does not reach the daemon: the default runs, and the trial stays pending.
    node.slot_power_cut();
    node.slot_boots_without_daemon(1);
    ok(&orion, names::REBOOT, b"");
    let tried = refused(&orion, names::SLOTS_REMOVE, b"slot-2");
    assert!(tried.detail.starts_with("-16:"), "{}", tried.detail);

    // A pinned slot is not removed.
    let pinned = bundle_for("slot-3", "3.0.0", None);
    assert_eq!(
        install(&orion, &pinned, true, true).outcome,
        Outcome::Succeeded
    );
    let finished = refused(&orion, names::SLOTS_REMOVE, b"slot-3");
    assert!(finished.detail.starts_with("-1:"), "{}", finished.detail);
    assert!(finished.detail.contains("pinned"), "{}", finished.detail);
}

#[test]
fn runs_started_counts_every_run_including_refused_ones() {
    let node = start("runs-started");
    let orion = node.connect("runs-started").expect("a session");
    assert_eq!(node.runs_started(names::SLOTS_INSTALL_FILE), 0);
    // A run for a file that is not there is asked for, and refused: it still counts.
    let missing = call(
        &orion,
        names::SLOTS_INSTALL_FILE,
        format!("{BUNDLE} false false").as_bytes(),
    )
    .expect("an outcome");
    assert_ne!(missing.outcome, Outcome::Succeeded);
    assert_eq!(node.runs_started(names::SLOTS_INSTALL_FILE), 1);
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    assert_eq!(node.runs_started(names::SLOTS_INSTALL_FILE), 2);
    assert_eq!(node.runs_started(names::SLOTS_TRIAL), 0);
}

#[test]
fn a_board_mismatch_at_install_is_minus_95_naming_both_boards_and_leaves_nothing() {
    let node = start("install-95");
    let orion = node.connect("install-95").expect("a session");
    let rootfs = b"rootfs of slot-2\n".repeat(64);
    let boot = bundle::boot_files(CMDLINE);
    let mut manifest = bundle::manifest("slot-2", "2.0.0", &rootfs, &boot);
    manifest["board"] = Value::String("another-board".into());
    let other = bundle::build(&manifest, None, &rootfs, &boot);
    let finished = install(&orion, &other, false, false);
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert_eq!(
        finished.detail,
        "-95: this slot is built for board another-board, and this is board simulated"
    );
    assert_eq!(slot_count(&list(&orion)), 1, "nothing is listed");
    assert_eq!(
        list(&orion)["state"],
        "error",
        "the state carries the refusal"
    );
}

#[test]
fn the_old_slot_key_member_is_refused_by_name_and_leaves_nothing() {
    let node = start("slot-key-member");
    let orion = node.connect("slot-key-member").expect("a session");
    let signer = SigningKey::from_bytes(&[7; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_sim_node::slots::TrustedKey {
            public: signer.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let rootfs = b"rootfs of slot-key\n".repeat(64);
    let boot = bundle::boot_files(CMDLINE);
    let manifest = bundle::manifest("slot-key", "2.0.0", &rootfs, &boot);
    let record = bundle::sign(&manifest, &signer);
    // A signed bundle with the old `slot.key` member: the key id is in the record, so the member
    // is outside the set and is refused by name, at its header.
    let mut parts = vec![
        ("slot.json".to_string(), manifest.to_string().into_bytes()),
        ("slot.sig".to_string(), record.to_vec()),
        ("slot.key".to_string(), bundle::key_id(&signer).into_bytes()),
        ("rootfs.erofs".to_string(), rootfs.clone()),
    ];
    parts.extend(
        boot.iter()
            .map(|(path, bytes)| (format!("boot/{path}"), bytes.to_vec())),
    );
    let bytes = bundle::tar_of(&parts);
    let finished = install(&orion, &bytes, false, false);
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert!(finished.detail.starts_with("-22:"), "{}", finished.detail);
    assert!(
        finished
            .detail
            .contains("slot.key is not part of a slot bundle"),
        "{}",
        finished.detail
    );
    assert_eq!(slot_count(&list(&orion)), 1, "nothing is listed");
}

/// A slot installed straight into the board's slot store, with the boot files its manifest lists.
fn slot_with_boot(id: &str, boot: &[(&str, &str)]) -> NewSlot {
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

/// A node whose default is `C` (previous `B`), both with boot files, and `C` running.
fn node_with_default_c(name: &str) -> (SimNode, Orion) {
    let node = start(name);
    let orion = node.connect(name).expect("a session");
    {
        let mut model = node.model();
        model
            .slots
            .install_done(
                slot_with_boot("B", &[("cmdline.txt", "c1"), ("kernel8.img", "k1")]),
                false,
                true,
            )
            .expect("installs B");
        model
            .slots
            .install_done(
                slot_with_boot("C", &[("cmdline.txt", "c1"), ("kernel8.img", "kc")]),
                false,
                true,
            )
            .expect("installs C");
        model.slots.default = Some("C".into());
        model.slots.previous = Some("B".into());
        model.slots.active = Some("C".into());
    }
    (node, orion)
}

#[test]
fn the_board_restart_refusal_reaches_atlas_with_the_boards_text_unchanged() {
    let (node, orion) = node_with_default_c("restart-refused-text");
    {
        let mut model = node.model();
        model
            .slots
            .set_boot_file("C", "kernel8.img", Some("tampered"))
            .expect("set");
        model
            .slots
            .set_boot_file("B", "cmdline.txt", Some("tampered"))
            .expect("set");
    }
    let finished = refused(&orion, names::REBOOT, b"");
    assert_eq!(finished.outcome, Outcome::Rejected, "{}", finished.detail);
    assert_eq!(
        finished.detail,
        "-1: C doesn't match its manifest (kernel8.img doesn't match its manifest hash), and its previous \"B\" doesn't either: the restart is refused"
    );
    // The board stays up on its default.
    assert_eq!(node.model().slots.active.as_deref(), Some("C"));
}

#[test]
fn the_restart_repoint_note_reaches_atlas_unchanged() {
    let (node, orion) = node_with_default_c("restart-repoint-note");
    node.model()
        .slots
        .set_boot_file("C", "kernel8.img", Some("tampered"))
        .expect("set");
    let finished = call(&orion, names::REBOOT, b"").expect("reboot answers");
    assert_eq!(finished.outcome, Outcome::Succeeded, "{}", finished.detail);
    assert_eq!(
        String::from_utf8(finished.output).expect("text"),
        "C doesn't match its manifest (kernel8.img doesn't match its manifest hash): B is the default now"
    );
    assert_eq!(node.model().slots.default.as_deref(), Some("B"));
}
