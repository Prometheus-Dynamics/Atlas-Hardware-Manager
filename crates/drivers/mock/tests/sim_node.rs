//! The simulated node, proven against Orion's own client and comm core.
#![cfg(target_os = "linux")]

use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use atlas_mock::bundle;
use atlas_mock::names::{self, SURFACE};
use atlas_mock::slots::code;
use atlas_mock::{Behaviour, LinkFault, SimNode, call};
use ed25519_dalek::SigningKey;
use orion::Orion;
use orion::actions::{ActionCall, Finished, Outcome};
use orion::topics::FieldId;
use serde_json::Value;

const CHUNK: usize = 4096;

fn start(name: &str) -> SimNode {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sim-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    SimNode::start(&dir).expect("the simulated node starts")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A bundle for slot `id`, signed by `key` when given.
fn bundle_for(id: &str, version: &str, key: Option<&SigningKey>) -> Vec<u8> {
    let rootfs = format!("rootfs of {id} at {version}\n")
        .repeat(900)
        .into_bytes();
    let boot: [(&str, &[u8]); 2] = [
        ("cmdline.txt", b"console=ttyS0 root=/dev/dm-0\n"),
        ("kernel.img", b"kernel bytes"),
    ];
    let manifest = bundle::manifest(id, version, &rootfs, &boot);
    let signature = key.map(|key| bundle::sign(&manifest, key));
    bundle::build(&manifest, signature.as_ref(), &rootfs, &boot)
}

/// Starts an install and streams the bundle in chunks. Returns how the install ended.
fn install(orion: &Orion, bundle: &[u8], pin: bool, keep_old: bool) -> Finished {
    let mut args = (bundle.len() as u64).to_le_bytes().to_vec();
    args.push(u8::from(pin));
    args.push(u8::from(keep_old));
    let mut run = orion
        .run(&ActionCall {
            name: names::SLOTS_INSTALL,
            target: atlas_mock::node::target(),
            args: &args,
            deadline_ms: 0,
            detached: false,
        })
        .expect("the install starts");
    for (index, chunk) in bundle.chunks(CHUNK).enumerate() {
        let mut write = ((index * CHUNK) as u64).to_le_bytes().to_vec();
        write.extend_from_slice(chunk);
        call(orion, names::SLOTS_WRITE, &write).expect("the write answers");
    }
    run.finish(Duration::from_secs(10))
        .expect("the install ends")
}

fn list(orion: &Orion) -> Value {
    let finished = call(orion, names::SLOTS_LIST, b"").expect("list answers");
    assert_eq!(finished.outcome, Outcome::Succeeded);
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

#[test]
fn every_surface_method_answers_and_its_result_is_right() {
    let node = start("surface");
    let orion = node.connect("surface").expect("a session");
    let mut exercised: Vec<&str> = Vec::new();

    // list: the board's one slot, running and default, good.
    let listing = list(&orion);
    assert_eq!(listing["slot_active"], "slot-1");
    assert_eq!(slot(&listing, "slot-1")["good"], "yes");
    exercised.push(names::SLOTS_LIST);

    // write before any install is refused and changes nothing.
    let stray = refused(&orion, names::SLOTS_WRITE, &[0_u8; 9]);
    assert_eq!(stray.outcome, Outcome::Rejected);
    exercised.push(names::SLOTS_WRITE);

    // install: a long job that ends with the new slot id. Installed slots start untried.
    let two = bundle_for("slot-2", "2.0.0", None);
    let done = install(&orion, &two, false, false);
    assert_eq!(done.outcome, Outcome::Succeeded, "{}", done.detail);
    assert_eq!(text(&done.output), "slot-2");
    assert_eq!(slot(&list(&orion), "slot-2")["good"], "untried");
    exercised.push(names::SLOTS_INSTALL);

    // trial restarts into slot-2; confirm makes it the default with slot-1 as previous.
    ok(&orion, names::SLOTS_TRIAL, b"id=slot-2");
    assert_eq!(list(&orion)["slot_active"], "slot-2");
    exercised.push(names::SLOTS_TRIAL);
    ok(&orion, names::SLOTS_CONFIRM, b"");
    let confirmed = list(&orion);
    assert_eq!(confirmed["slot_default"], "slot-2");
    assert_eq!(confirmed["slot_previous"], "slot-1");
    exercised.push(names::SLOTS_CONFIRM);

    // set_default needs a good slot: slot-1 is, so it can be made the default again.
    ok(&orion, names::SLOTS_SET_DEFAULT, b"id=slot-1");
    assert_eq!(list(&orion)["slot_default"], "slot-1");
    refused(&orion, names::SLOTS_SET_DEFAULT, b"id=slot-9");
    exercised.push(names::SLOTS_SET_DEFAULT);

    // pin: a pinned slot will not be removed.
    ok(&orion, names::SLOTS_PIN, b"id=slot-2\npinned=true");
    assert_eq!(slot(&list(&orion), "slot-2")["pinned"], true);
    let pinned = refused(&orion, names::SLOTS_REMOVE, b"id=slot-2");
    assert_eq!(pinned.outcome, Outcome::Rejected);
    exercised.push(names::SLOTS_PIN);

    // remove: the default slot is busy; an unknown id is not found.
    refused(&orion, names::SLOTS_REMOVE, b"id=slot-1");
    refused(&orion, names::SLOTS_REMOVE, b"id=..");
    ok(&orion, names::SLOTS_PIN, b"id=slot-2\npinned=false");
    // slot-2 is still the running slot until the next boot, so it is busy until then.
    refused(&orion, names::SLOTS_REMOVE, b"id=slot-2");
    ok(&orion, names::REBOOT, b"");
    ok(&orion, names::SLOTS_REMOVE, b"id=slot-2");
    assert!(slot_missing(&list(&orion), "slot-2"));
    exercised.push(names::SLOTS_REMOVE);

    // set_keep: zero keeps all; any number runs housekeeping.
    ok(&orion, names::SLOTS_SET_KEEP, b"keep=0");
    ok(&orion, names::SLOTS_SET_KEEP, b"keep=3");
    exercised.push(names::SLOTS_SET_KEEP);

    // reboot restarts into the default.
    ok(&orion, names::REBOOT, b"");
    assert_eq!(list(&orion)["slot_active"], "slot-1");
    exercised.push(names::REBOOT);

    let mut surface: Vec<&str> = SURFACE.iter().map(|method| method.action).collect();
    surface.sort_unstable();
    exercised.sort_unstable();
    assert_eq!(
        exercised, surface,
        "every method of the I5 surface is exercised"
    );
}

fn slot_missing(listing: &Value, id: &str) -> bool {
    !listing["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .any(|slot| slot["id"] == id)
}

#[test]
fn the_board_answers_its_own_actions_and_data_partition() {
    let node = start("own");
    let orion = node.connect("own").expect("a session");
    assert_eq!(ok(&orion, names::LOCATE, b"").output, b"blinking");
    assert_eq!(ok(&orion, names::SELF_TEST, b"").output, b"passed");
    assert_eq!(ok(&orion, names::SET_CLOCK, b"").output, b"clock set");
    assert!(node.model().clock_offset_s.is_none());
    assert!(text(&ok(&orion, names::STATUS, b"").output).contains("running=slot-1"));
    assert!(text(&ok(&orion, names::SUPPORT_BUNDLE, b"").output).contains("slot-1"));

    let backup = ok(&orion, names::DATA_BACKUP, b"").output;
    ok(&orion, names::FS_WRITE, b"path=notes.txt\nhello");
    assert_eq!(
        ok(&orion, names::FS_READ, b"path=notes.txt").output,
        b"hello"
    );
    assert!(text(&ok(&orion, names::FS_LIST, b"path=").output).contains("notes.txt"));
    ok(&orion, names::FS_REMOVE, b"path=notes.txt");
    refused(&orion, names::FS_READ, b"path=notes.txt");
    ok(&orion, names::DATA_RESTORE, &backup);
    assert!(node.model().data.contains_key("config/board.toml"));
    refused(&orion, names::DATA_RESTORE, b"not a backup");
    assert!(
        node.model().data.contains_key("config/board.toml"),
        "a bad restore changes nothing"
    );
}

#[test]
fn a_refused_run_changes_nothing() {
    let node = start("refuse");
    let orion = node.connect("refuse").expect("a session");
    node.script().push_action(
        names::SLOTS_REMOVE,
        Behaviour::Refuse("held by the test".into()),
    );
    let finished = call(&orion, names::SLOTS_REMOVE, b"id=slot-1").expect("an outcome");
    assert_eq!(finished.outcome, Outcome::Rejected);
    assert_eq!(finished.detail, "held by the test");
    assert_eq!(list(&orion)["slot_default"], "slot-1");
}

#[test]
fn a_failed_run_changes_nothing() {
    let node = start("fail");
    let orion = node.connect("fail").expect("a session");
    node.script().push_action(
        names::SLOTS_CONFIRM,
        Behaviour::Fail("the board did not answer".into()),
    );
    let finished = call(&orion, names::SLOTS_CONFIRM, b"").expect("an outcome");
    assert_eq!(finished.outcome, Outcome::Failed);
    assert_eq!(list(&orion)["slot_default"], "slot-1");
}

#[test]
fn a_stalled_self_test_is_cancelled_with_no_change() {
    let node = start("stall");
    let orion = node.connect("stall").expect("a session");
    node.script()
        .push_action(names::SELF_TEST, Behaviour::Stall);
    let mut run = orion
        .run(&ActionCall {
            name: names::SELF_TEST,
            target: atlas_mock::node::target(),
            args: b"",
            deadline_ms: 0,
            detached: false,
        })
        .expect("the run starts");
    std::thread::sleep(Duration::from_millis(60));
    run.cancel().expect("cancel is sent");
    assert_eq!(
        run.finish(Duration::from_secs(5)).expect("it ends").outcome,
        Outcome::Cancelled
    );
    assert!(node.model().slots.trial.is_none());
}

#[test]
fn cancel_discards_a_partly_written_install() {
    let node = start("cancel");
    let orion = node.connect("cancel").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    let mut args = (two.len() as u64).to_le_bytes().to_vec();
    args.extend_from_slice(&[0, 0]);
    let mut run = orion
        .run(&ActionCall {
            name: names::SLOTS_INSTALL,
            target: atlas_mock::node::target(),
            args: &args,
            deadline_ms: 0,
            detached: false,
        })
        .expect("the install starts");
    let mut first = 0_u64.to_le_bytes().to_vec();
    first.extend_from_slice(&two[..CHUNK]);
    ok(&orion, names::SLOTS_WRITE, &first);
    run.cancel().expect("cancel is sent");
    assert_eq!(
        run.finish(Duration::from_secs(5)).expect("it ends").outcome,
        Outcome::Cancelled
    );
    let listing = list(&orion);
    assert_eq!(listing["state"], "cancelled");
    assert!(
        slot_missing(&listing, "slot-2"),
        "nothing was written as a slot"
    );
    refused(&orion, names::SLOTS_WRITE, &first);
}

#[test]
fn a_starter_that_disconnects_has_its_install_cancelled() {
    let node = start("disconnect");
    let starter = node.connect("starter").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    let mut args = (two.len() as u64).to_le_bytes().to_vec();
    args.extend_from_slice(&[0, 0]);
    let run = starter
        .run(&ActionCall {
            name: names::SLOTS_INSTALL,
            target: atlas_mock::node::target(),
            args: &args,
            deadline_ms: 0,
            detached: false,
        })
        .expect("the install starts");
    let mut first = 0_u64.to_le_bytes().to_vec();
    first.extend_from_slice(&two[..CHUNK]);
    ok(&starter, names::SLOTS_WRITE, &first);
    drop(run);
    drop(starter);

    let other = node.connect("after").expect("a session");
    let deadline = Instant::now() + Duration::from_secs(5);
    while list(&other)["state"] != "cancelled" {
        assert!(
            Instant::now() < deadline,
            "the install was not cancelled when its starter left"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    refused(&other, names::SLOTS_WRITE, &first);
}

#[test]
fn a_service_restart_loses_the_running_install_and_a_new_one_starts_clean() {
    let node = start("restart");
    let orion = node.connect("restart").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    let mut args = (two.len() as u64).to_le_bytes().to_vec();
    args.extend_from_slice(&[0, 0]);
    let mut run = orion
        .run(&ActionCall {
            name: names::SLOTS_INSTALL,
            target: atlas_mock::node::target(),
            args: &args,
            deadline_ms: 0,
            detached: false,
        })
        .expect("the install starts");
    let mut first = 0_u64.to_le_bytes().to_vec();
    first.extend_from_slice(&two[..CHUNK]);
    ok(&orion, names::SLOTS_WRITE, &first);

    node.restart_daemon();
    let lost = run.finish(Duration::from_secs(5)).expect("it ends");
    assert_eq!(lost.outcome, Outcome::Failed, "{}", lost.detail);
    assert!(lost.detail.starts_with("-4"), "{}", lost.detail);
    let listing = list(&orion);
    assert_eq!(listing["state"], "error");
    assert_eq!(
        listing["error"],
        "an install was interrupted: the service restarted"
    );
    refused(&orion, names::SLOTS_WRITE, &[0_u8; 8]);

    // Nothing resumes: a fresh install of the same bundle starts from zero and lands.
    let again = install(&orion, &two, false, false);
    assert_eq!(again.outcome, Outcome::Succeeded, "{}", again.detail);
    assert_eq!(list(&orion)["state"], "idle");
}

#[test]
fn a_scripted_restart_loses_jobs_at_its_step() {
    let node = start("scripted-restart");
    let orion = node.connect("scripted").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    node.script()
        .push_action(names::STATUS, Behaviour::RestartDaemon);
    let finished = call(&orion, names::STATUS, b"").expect("an outcome");
    assert_eq!(finished.outcome, Outcome::Failed);
    let landed = install(&orion, &two, false, false);
    assert_eq!(landed.outcome, Outcome::Succeeded, "{}", landed.detail);
}

#[test]
fn a_trial_that_does_not_confirm_falls_back_and_marks_the_slot_failed() {
    let node = start("trial");
    let orion = node.connect("trial").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );

    ok(&orion, names::SLOTS_TRIAL, b"id=slot-2");
    assert_eq!(list(&orion)["slot_active"], "slot-2");
    ok(&orion, names::REBOOT, b"");
    let fell_back = list(&orion);
    assert_eq!(
        fell_back["slot_active"], "slot-1",
        "fell back to the default"
    );
    assert_eq!(fell_back["state"], "rolled-back");
    assert_eq!(slot(&fell_back, "slot-2")["failed"], true);
    assert_eq!(slot(&fell_back, "slot-2")["good"], "no");

    // A failed slot is not made the default unless forced.
    refused(&orion, names::SLOTS_SET_DEFAULT, b"id=slot-2");
    refused(&orion, names::SLOTS_TRIAL, b"id=slot-2");
    ok(&orion, names::SLOTS_TRIAL, b"id=slot-2\nforce=true");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    assert_eq!(list(&orion)["slot_default"], "slot-2");
}

#[test]
fn a_trial_with_keep_default_leaves_the_default_alone_on_confirm() {
    let node = start("keep-default");
    let orion = node.connect("keep-default").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    ok(&orion, names::SLOTS_TRIAL, b"id=slot-2\nkeep_default=true");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    let listing = list(&orion);
    assert_eq!(listing["slot_default"], "slot-1");
    assert_eq!(listing["slot_active"], "slot-2");
    assert_eq!(slot(&listing, "slot-2")["good"], "yes");
}

#[test]
fn a_trial_confirmed_becomes_the_default_and_survives_a_reboot() {
    let node = start("confirm");
    let orion = node.connect("confirm").expect("a session");
    let two = bundle_for("slot-2", "2.0.0", None);
    assert_eq!(
        install(&orion, &two, false, false).outcome,
        Outcome::Succeeded
    );
    ok(&orion, names::SLOTS_TRIAL, b"id=slot-2");
    ok(&orion, names::SLOTS_CONFIRM, b"");
    ok(&orion, names::REBOOT, b"");
    let listing = list(&orion);
    assert_eq!(listing["slot_default"], "slot-2");
    assert_eq!(listing["slot_active"], "slot-2");
    assert_eq!(slot(&listing, "slot-2")["failed"], false);
}

#[test]
fn housekeeping_keeps_the_last_n_and_the_pinned_ones_and_keep_old_skips_it() {
    let node = start("housekeeping");
    let orion = node.connect("housekeeping").expect("a session");
    ok(&orion, names::SLOTS_SET_KEEP, b"keep=2");
    let pinned = bundle_for("pinned", "1.0.0", None);
    assert_eq!(
        install(&orion, &pinned, true, false).outcome,
        Outcome::Succeeded
    );
    for version in 2..=5 {
        let id = format!("slot-{}", version + 1);
        let landed = install(
            &orion,
            &bundle_for(&id, &format!("{version}.0.0"), None),
            false,
            false,
        );
        assert_eq!(landed.outcome, Outcome::Succeeded, "{}", landed.detail);
    }
    let kept: Vec<String> = list(&orion)["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .map(|slot| slot["id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        kept.contains(&"pinned".to_string()),
        "a pinned slot stays: {kept:?}"
    );
    assert!(
        kept.contains(&"slot-1".to_string()),
        "the running and default slot stays: {kept:?}"
    );
    assert!(kept.len() <= 3, "keep-last and the pin: {kept:?}");

    // keep_old on an install skips housekeeping for it.
    let before = kept.len();
    assert_eq!(
        install(&orion, &bundle_for("kept-old", "9.0.0", None), false, true).outcome,
        Outcome::Succeeded
    );
    assert_eq!(
        list(&orion)["slots"].as_array().expect("slots").len(),
        before + 1
    );
}

#[test]
fn a_bundle_is_checked_and_a_bad_one_leaves_nothing() {
    let node = start("bundles");
    let orion = node.connect("bundles").expect("a session");
    let good = bundle_for("slot-2", "2.0.0", None);

    // A tar with a symlink member.
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    builder
        .append_link(&mut header, "rootfs.erofs", "/etc/passwd")
        .expect("a link");
    let link = builder.into_inner().expect("a tar");
    let cases: Vec<(&str, Vec<u8>, i32)> = vec![
        ("a symlink", link, code::INVALID_ARGUMENT),
        (
            "a broken tar",
            b"not a tar".to_vec(),
            code::INVALID_ARGUMENT,
        ),
        (
            "an unknown member",
            with_extra_member(&good),
            code::INVALID_ARGUMENT,
        ),
        (
            "a running id",
            bundle_for("slot-1", "2.0.0", None),
            code::BUSY,
        ),
    ];
    for (what, bytes, expected) in cases {
        let finished = install(&orion, &bytes, false, false);
        assert_ne!(
            finished.outcome,
            Outcome::Succeeded,
            "{what} must be refused"
        );
        assert!(
            finished.detail.starts_with(&format!("{expected}:")),
            "{what}: {}",
            finished.detail
        );
    }
    let listing = list(&orion);
    assert_eq!(
        listing["slots"].as_array().expect("slots").len(),
        1,
        "nothing was left behind"
    );
}

/// The good bundle with one more member the manifest does not list.
fn with_extra_member(good: &[u8]) -> Vec<u8> {
    let mut archive = tar::Archive::new(Cursor::new(good));
    let mut builder = tar::Builder::new(Vec::new());
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        let path = entry.path().expect("path").into_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).expect("bytes");
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, bytes.as_slice())
            .expect("member");
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(3);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "boot/extra.txt", &b"abc"[..])
        .expect("extra");
    builder.into_inner().expect("tar")
}

#[test]
fn security_off_shows_signatures_as_unverified_and_installs_them_all() {
    let node = start("security-off");
    let orion = node.connect("security-off").expect("a session");
    let key = SigningKey::from_bytes(&[7; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_mock::slots::TrustedKey {
            public: key.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let signed = bundle_for("signed", "2.0.0", Some(&key));
    assert_eq!(
        install(&orion, &signed, false, false).outcome,
        Outcome::Succeeded
    );
    let unsigned = bundle_for("unsigned", "3.0.0", None);
    assert_eq!(
        install(&orion, &unsigned, false, false).outcome,
        Outcome::Succeeded
    );
    let listing = list(&orion);
    assert_eq!(slot(&listing, "signed")["signed"], "trusted");
    assert_eq!(slot(&listing, "signed")["signer"], "Test Owner");
    assert_eq!(slot(&listing, "unsigned")["signed"], "unverified");
}

#[test]
fn security_on_refuses_unsigned_and_untrusted_bundles_and_keeps_nothing() {
    let node = start("security-on");
    let orion = node.connect("security-on").expect("a session");
    node.model().slots.require_signed = true;

    // No trusted keys at all: the install does not start.
    let refusal = install(&orion, &bundle_for("slot-2", "2.0.0", None), false, false);
    assert_eq!(refusal.outcome, Outcome::Rejected);
    assert!(refusal.detail.starts_with("-1:"), "{}", refusal.detail);

    let trusted = SigningKey::from_bytes(&[7; 32]);
    let stranger = SigningKey::from_bytes(&[9; 32]);
    node.model()
        .slots
        .trusted
        .push(atlas_mock::slots::TrustedKey {
            public: trusted.verifying_key().to_bytes(),
            owner: "Test Owner".into(),
        });
    let unsigned = install(&orion, &bundle_for("unsigned", "2.0.0", None), false, false);
    assert!(unsigned.detail.starts_with("-1:"), "{}", unsigned.detail);
    let untrusted = install(
        &orion,
        &bundle_for("untrusted", "2.0.0", Some(&stranger)),
        false,
        false,
    );
    assert!(untrusted.detail.starts_with("-1:"), "{}", untrusted.detail);
    assert_eq!(
        list(&orion)["slots"].as_array().expect("slots").len(),
        1,
        "nothing left behind"
    );

    let good = install(
        &orion,
        &bundle_for("trusted", "2.0.0", Some(&trusted)),
        false,
        false,
    );
    assert_eq!(good.outcome, Outcome::Succeeded, "{}", good.detail);
    assert_eq!(slot(&list(&orion), "trusted")["signed"], "trusted");
}

#[test]
fn an_install_refused_for_room_says_so() {
    let node = start("room");
    let orion = node.connect("room").expect("a session");
    node.model().slots.free_bytes = 100;
    let finished = install(&orion, &bundle_for("slot-2", "2.0.0", None), false, false);
    assert_ne!(finished.outcome, Outcome::Succeeded);
    assert!(finished.detail.starts_with("-28:"), "{}", finished.detail);
}

#[test]
fn readings_come_with_their_units_on_a_topic() {
    let node = start("readings");
    let orion = node.connect("reader").expect("a session");
    let mut subscriber = orion.subscribe(names::READINGS).expect("a subscription");
    let deadline = Instant::now() + Duration::from_secs(5);
    let temperature = loop {
        subscriber
            .wait(Duration::from_millis(50))
            .expect("the topic waits");
        if let Some(sample) = subscriber.latest(FieldId(0)) {
            break f64::from_le_bytes(sample.payload.try_into().expect("8 bytes"));
        }
        assert!(Instant::now() < deadline, "no temperature sample arrived");
    };
    assert!(
        (40.0..=50.0).contains(&temperature),
        "temperature {temperature}"
    );
    let mut hardware = orion
        .subscribe(names::HARDWARE)
        .expect("the hardware topic");
    hardware
        .wait(Duration::from_secs(5))
        .expect("the description arrives");
    let description = hardware
        .latest(FieldId(0))
        .expect("one description")
        .payload
        .to_vec();
    assert!(
        text(&description).contains("model="),
        "{}",
        text(&description)
    );
}

#[test]
fn a_file_dropped_mid_transfer_resumes_and_lands_whole() {
    let node = start("files");
    let data: Vec<u8> = (0..300_000_u32).map(|i| (i % 251) as u8).collect();
    let destination = "images/system.img";

    // The link cuts the first connection after 100 kB of the transfer.
    node.script().push_link(LinkFault::Drop {
        after_bytes: 100_000,
    });
    let first = node
        .connect_through_link("uploader")
        .expect("a session through the link");
    let cut = first
        .files()
        .push(destination, &mut Cursor::new(&data), |_, _| {});
    assert!(cut.is_err(), "the cut link fails the push");
    assert!(
        node.shelf()
            .committed
            .lock()
            .unwrap()
            .get(destination)
            .is_none()
    );

    // A fresh connection resumes from where the board has the bytes.
    let second = node
        .connect_through_link("uploader")
        .expect("a session through the link");
    let transfer = second
        .files()
        .push(destination, &mut Cursor::new(&data), |_, _| {})
        .expect("the resumed push");
    assert_eq!(transfer.received, data.len() as u64);
    let committed = node.shelf().committed.lock().unwrap();
    assert_eq!(committed.get(destination), Some(&data));
}

#[test]
fn a_stalled_link_holds_then_carries_on() {
    let node = start("link-stall");
    let pause = Duration::from_millis(300);
    node.script().push_link(LinkFault::Stall { pause });
    let started = Instant::now();
    let orion = node
        .connect_through_link("slow")
        .expect("a session through the link");
    let finished = call(&orion, names::STATUS, b"").expect("an answer after the stall");
    assert_eq!(finished.outcome, Outcome::Succeeded);
    assert!(started.elapsed() >= pause, "the stall held the connection");
}

#[test]
fn the_board_is_reachable_on_the_socket_and_the_network_alike() {
    let node = start("both-ways");
    let direct = node.connect("direct").expect("a session on the socket");
    let network = node
        .connect_through_link("network")
        .expect("a session through the link");
    let a = call(&direct, names::STATUS, b"").expect("status");
    let b = call(&network, names::STATUS, b"").expect("status");
    assert_eq!(a.outcome, Outcome::Succeeded);
    assert_eq!(text(&a.output), text(&b.output));
}
