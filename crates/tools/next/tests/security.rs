//! T3's security-on proofs: a simulated board with security on (`SimNode::start_secured`), where a
//! PC's key is trusted only when the board's trust list says so. Run through the same `execute`
//! the binary runs. Every identity file is under this test's scratch directory.

mod common;

use std::path::Path;

use atlas_next::identity;
use atlas_sim_node::SimNode;
use common::{env, kind, run, scratch};
use orion::node::OperatorState;

fn direct(node: &SimNode) -> String {
    format!("orion+tcp://127.0.0.1:{}", node.network().port())
}

/// The board's own trust list is the one under test: a secured node in `dir`.
fn secured(dir: &Path) -> SimNode {
    SimNode::start_secured(&dir.join("node"), &kind(), None).expect("the secured node starts")
}

/// The public key of the identity file at `path` (made if absent): the key the board's trust list
/// holds, as the board sees this PC.
fn public_of(path: &Path) -> [u8; 32] {
    let secret = identity::load_or_create(path)
        .expect("the identity is made or read")
        .secret;
    identity::public_key(&secret)
}

/// Proof 1: on a secured board, a PC whose key is not enrolled gets Orion's refusal of the call
/// (`denied`) and the one line that says what to do, exit 1. No security-off line.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unenrolled_key_is_denied_and_told_to_enrol() {
    let dir = scratch("sec-on-denied");
    let node = secured(&dir);
    let address = direct(&node);
    let env = env(&dir.join("state"));

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "pin works on a secured board: {out}");

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("denied"), "Orion's words are shown: {out}");
    assert!(
        out.lines().any(|line| line
            == format!(
                "this PC's key is not enrolled on this board: run atlas-next enrol {address}"
            )),
        "{out}"
    );
    assert!(!out.contains("security off"), "{out}");
}

/// Proof 2: `enrol` on a secured board prints the pending sentence, and the board's trust list
/// holds this PC's key as pending. Nothing is granted by it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrol_leaves_the_key_pending_on_the_trust_list() {
    let dir = scratch("sec-on-pending");
    let node = secured(&dir);
    let address = direct(&node);
    let env = env(&dir.join("state"));
    let file = dir.join("state").join("identity.key");

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    let (code, out) = run(&env, &["enrol", &address]).await;
    assert_eq!(code, 0, "{out}");
    let key = public_of(&file);
    let fingerprint = identity::fingerprint(&key);
    assert!(
        out.lines().any(|line| line
            == format!(
                "your key {fingerprint} is now pending on this board: the board's owner must approve it before this PC can do anything there"
            )),
        "{out}"
    );
    assert_eq!(node.trusted(&key), Some(OperatorState::Pending), "{out}");

    // Pending grants nothing: a list is still denied.
    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("denied"), "{out}");
}

/// Proof 3: once an administrator on the board trusts the key (the board's own store makes it an
/// operator with every action), `list` works and no security line is printed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_enrolled_key_lists_and_no_security_line_is_printed() {
    let dir = scratch("sec-on-enrolled");
    let node = secured(&dir);
    let address = direct(&node);
    let env = env(&dir.join("state"));
    let file = dir.join("state").join("identity.key");

    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");
    let (code, out) = run(&env, &["enrol", &address]).await;
    assert_eq!(code, 0, "{out}");

    let key = public_of(&file);
    node.grant_as_root(key, "test pc")
        .expect("root trusts the key");
    assert_eq!(node.trusted(&key), Some(OperatorState::Enrolled));

    let (code, out) = run(&env, &["slots", &address, "list"]).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("running "), "no security line first: {out}");
    assert!(!out.contains("security"), "{out}");
}

/// Proof 4: a revoked key's `enrol` prints Orion's own refusal of the session, exit 1. The board
/// turns a revoked key away at the handshake, so the refusal is typed (`ClientError::Denied`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_key_enrol_prints_orions_refusal() {
    let dir = scratch("sec-on-revoked");
    let node = secured(&dir);
    let address = direct(&node);
    let env = env(&dir.join("state"));
    let file = dir.join("revoked.key");
    let file_text = file.display().to_string();

    let (code, out) = run(&env, &["slots", &address, "pin", "--identity", &file_text]).await;
    assert_eq!(code, 0, "{out}");

    node.revoke(public_of(&file)).expect("root revokes the key");

    let (code, out) = run(&env, &["enrol", &address, "--identity", &file_text]).await;
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("the node refused this client's key"),
        "Orion's words: {out}"
    );
}
