//! T3's proofs: the PC's one client identity (its file, its command, the sessions that sign with
//! it) and the security-off rules, run through the same `execute` the binary runs. Every file is
//! under this test's scratch directory: nothing touches the real config directory.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use atlas_next::identity;
use atlas_session::{Presence, Sessions};
use atlas_sim_node::SimNode;
use common::{YieldPause, env, identity as board_identity, run, scratch, start_node};
use orion::Address;

fn direct(node: &SimNode) -> String {
    format!("orion+tcp://127.0.0.1:{}", node.network().port())
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("the file has metadata")
        .permissions()
        .mode()
        & 0o777
}

/// Proof 1: a fresh path makes a 0600 file in a 0700 directory; the second call returns the same
/// identity without making another; a 0644 file is refused; a damaged file is refused and left as
/// it is.
#[test]
fn the_identity_file_is_made_once_private_and_read_back() {
    let dir = scratch("ident-file");
    let path = dir.join("atlas").join("identity.key");

    let first = identity::load_or_create(&path).expect("the identity is made");
    assert!(first.created);
    assert_eq!(mode(&path), 0o600, "the file is owner-only");
    assert_eq!(
        mode(path.parent().expect("a directory")),
        0o700,
        "the directory is owner-only"
    );

    let second = identity::load_or_create(&path).expect("the identity is read");
    assert!(!second.created, "the second call makes nothing");
    assert_eq!(second.secret, first.secret, "the same identity comes back");

    // A file readable by others is refused, named by its mode, and not changed.
    let text = std::fs::read(&path).expect("the file reads");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let why = identity::load_or_create(&path).expect_err("a 0644 file is refused");
    assert!(why.contains("644") && why.contains("chmod 600"), "{why}");
    assert_eq!(std::fs::read(&path).expect("reads"), text, "left as it is");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

    // A damaged file (63 hex characters) is refused, its content is not echoed, and it stays.
    let damaged = "a".repeat(63) + "\n";
    std::fs::write(&path, &damaged).expect("the damaged file is written");
    let why = identity::load_or_create(&path).expect_err("a damaged file is refused");
    assert!(
        !why.contains(&"a".repeat(63)),
        "the content is not echoed: {why}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("reads"),
        damaged,
        "left as it is"
    );
}

/// Proof 3: `identity --identity <file>` prints the fingerprint and the path; a second run prints
/// the same fingerprint and makes nothing new.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_identity_command_prints_its_fingerprint_and_path_and_repeats() {
    let dir = scratch("ident-cmd");
    let file = dir.join("id.key");
    let env = env(&dir.join("state"));
    let file_text = file.display().to_string();

    let (code, out) = run(&env, &["identity", "--identity", &file_text]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("created this PC's client identity"), "{out}");
    let line = out
        .lines()
        .find(|line| line.starts_with("identity sha256:"))
        .expect("the fingerprint line")
        .to_owned();
    assert!(out.contains(&file_text), "the path is printed: {out}");

    let (code, again) = run(&env, &["identity", "--identity", &file_text]).await;
    assert_eq!(code, 0, "{again}");
    assert!(!again.contains("created"), "{again}");
    assert!(
        again.lines().any(|found| found == line),
        "the same fingerprint: {again}"
    );
}

/// The fingerprint `next` prints for an identity is the one Orion prints for a node: a simulated
/// node's own fingerprint is the fingerprint of its own key, in the same form.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fingerprint_is_the_form_orion_prints_for_a_node() {
    let dir = scratch("ident-form");
    let node = start_node(&dir, None);
    let board = board_identity(&node).await;
    assert_eq!(identity::fingerprint(&board.public_key), board.fingerprint);
    assert!(board.fingerprint.starts_with("sha256:"));
}

/// Proof 2 (part): two sessions with the same identity both connect to a node and are answered by
/// it. The sim does not show the client's key on its side, so this proves the sessions connect with
/// the identity; that the key itself is the same is proven by the identity file's test above and
/// by the connect code (`Identity::from_secret(&self.secret)` in `monitor.rs`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_sessions_with_one_identity_connect_to_the_same_node() {
    let dir = scratch("ident-sessions");
    let node = start_node(&dir, None);
    let address: Address = direct(&node).parse().expect("the address parses");
    let board = board_identity(&node).await;
    let secret = identity::load_or_create(&dir.join("id.key"))
        .expect("the identity is made")
        .secret;

    for _ in 0..2 {
        let sessions = Sessions::with_identity(Arc::new(YieldPause), secret);
        let handle = sessions.handle(&address).expect("the address is watched");
        let mut presence = handle.watch();
        let found = presence
            .wait_for(|presence| matches!(presence, Presence::Found(_)))
            .await
            .expect("the session stays up");
        match &*found {
            Presence::Found(node) => assert_eq!(node.public_key, board.public_key),
            _ => unreachable!("waited for Found"),
        }
    }
}

/// Security off (the sim's default) changes nothing about `pin` and `fingerprint`: they work as on
/// any board. `enrol` says there is nothing to enrol, exit 1. Nothing is refused for the mode.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_security_off_board_pins_and_fingerprints_like_any_board_and_enrol_says_so() {
    let dir = scratch("sec-off-pin");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let board = board_identity(&node).await;
    let env = env(&dir.join("state"));

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&board.fingerprint), "{out}");

    let (code, out) = run(&env, &["slots", &address, "fingerprint"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("pinned: this is the key"), "{out}");

    let (code, out) = run(&env, &["enrol", &address]).await;
    assert_eq!(code, 1, "{out}");
    assert_eq!(
        out.trim_end(),
        "this board runs with security off: there is nothing to enrol"
    );
}

/// Security off: a board command prints exactly one first line, the security line, then its normal
/// output; a board never seen still refuses (the pin rule applies to every board).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_security_off_board_lists_with_exactly_one_first_line() {
    let dir = scratch("sec-off-list");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let env = env(&dir.join("state"));

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 3, "never seen: {out}");
    assert_eq!(
        out.lines().next(),
        Some("security off: the board's key is checked, the session is not signed"),
        "{out}"
    );

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 0, "{out}");
    let mut lines = out.lines();
    assert_eq!(
        lines.next(),
        Some("security off: the board's key is checked, the session is not signed"),
        "{out}"
    );
    assert!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("running ")),
        "{out}"
    );
    assert_eq!(out.matches("security off").count(), 1, "said once: {out}");
}
