//! The six proofs of T2, run through `execute`, the function the binary runs.
//!
//! Every test makes its own temporary directory (`tempfile`, which is `std::env::temp_dir()`:
//! `$TMPDIR`, else `/tmp`). Each one first checks that this directory is outside any git work tree,
//! and stops the test when it is not, so a seed file is never written inside a repository. Every
//! seed here is made by the run itself or read back from a file the run wrote; none is a real key.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use atlas_bundle::{SEED_LEN, Seed, sign_slot_record, slot_message};
use atlas_next::{Env, bench_key, execute};
use atlas_release::PublicKey;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// The owner text the key-list line must hold.
const OWNER: &str = "bench key, not for any released image";

/// A temporary directory that is outside any git work tree, or the test stops here.
fn fresh() -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("atlas-next-bench-key-")
        .tempdir()
        .expect("a temporary directory");
    if let Some(top) = work_tree_of(dir.path()) {
        panic!(
            "the temporary directory {} is inside the git work tree {}: set TMPDIR to a directory outside any repository",
            dir.path().display(),
            top.display()
        );
    }
    dir
}

/// The same walk the command makes: the first directory of `path` and its parents with `.git`.
fn work_tree_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| fs::symlink_metadata(dir.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

/// Runs `atlas-next` with `args` through `execute`: the exit status and the output text.
async fn run(args: &[&str]) -> (i32, String) {
    let env = Env::new(None);
    let args: Vec<String> = std::iter::once("bench-key")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    let mut out = Vec::new();
    let code = execute(&args, &env, &mut out).await;
    (code, String::from_utf8(out).expect("utf-8 output"))
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("the file exists")
        .permissions()
        .mode()
        & 0o777
}

/// The printed value of a labelled line, e.g. `key id: <hex>`.
fn line_value<'a>(out: &'a str, label: &str) -> &'a str {
    out.lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("no `{label}` line in:\n{out}"))
}

fn hex_decode(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}

/// The 64 lowercase hex characters of the public key, read from the seed the run wrote.
fn public_hex_of(seed: &Seed) -> String {
    seed.public_key()
        .to_string()
        .strip_prefix("ed25519:")
        .expect("the display form")
        .to_owned()
}

/// Proof 1: a run in a fresh directory outside any repository writes the three files with their
/// modes, the seed reads back to the printed key id, the public key file is the seed's public key,
/// the key-list line is exact, and the key id is SHA-256 of the 32 public key bytes.
#[tokio::test]
async fn a_fresh_directory_gets_the_three_files_with_the_stated_modes_and_contents() {
    let dir = fresh();
    let (code, out) = run(&[dir.path().to_str().expect("utf-8 path")]).await;
    assert_eq!(code, 0, "{out}");

    let seed_path = dir.path().join(bench_key::SEED_FILE);
    let public_path = dir.path().join(bench_key::PUBLIC_FILE);
    let keys_path = dir.path().join(bench_key::KEYS_FILE);
    assert_eq!(mode(&seed_path), 0o600);
    assert_eq!(mode(&public_path), 0o644);
    assert_eq!(mode(&keys_path), 0o644);

    let printed_id = line_value(&out, "key id: ");
    let seed = Seed::read(&seed_path).expect("the written seed reads back");
    assert_eq!(seed.key_id_hex(), printed_id);

    let public = fs::read_to_string(&public_path).expect("bench.pub");
    let public_hex = public_hex_of(&seed);
    assert_eq!(public, format!("{public_hex}\n"));
    assert_eq!(public_hex.len(), 64);
    assert!(
        public_hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );

    let keys = fs::read_to_string(&keys_path).expect("bench.keys");
    assert_eq!(keys, format!("{public_hex} {OWNER}\n"));

    let digest = Sha256::digest(hex_decode(&public_hex));
    let digest_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        digest_hex, printed_id,
        "the key id is SHA-256 of the public key bytes"
    );

    // The printed paths are the three files, absolute and in the directory the run was given.
    let canonical = fs::canonicalize(dir.path()).expect("canonical");
    assert_eq!(
        line_value(&out, "seed file: "),
        canonical.join(bench_key::SEED_FILE).display().to_string()
    );
    assert_eq!(
        line_value(&out, "public key file: "),
        canonical.join(bench_key::PUBLIC_FILE).display().to_string()
    );
    assert_eq!(
        line_value(&out, "key list file: "),
        canonical.join(bench_key::KEYS_FILE).display().to_string()
    );
    assert_eq!(
        fs::read_dir(dir.path()).expect("list").count(),
        3,
        "nothing else was written"
    );
}

/// Proof 2: the text and JSON outputs hold no part of the seed: neither its hex, nor its first or
/// last 8 characters.
#[tokio::test]
async fn the_output_holds_no_part_of_the_seed() {
    let dir = fresh();
    let (code, text) = run(&[dir.path().to_str().expect("utf-8 path")]).await;
    assert_eq!(code, 0, "{text}");
    let seed_hex = fs::read_to_string(dir.path().join(bench_key::SEED_FILE)).expect("seed");
    let seed_hex = seed_hex.trim_end();
    assert_eq!(seed_hex.len(), 64);

    let dir_json = fresh();
    let (code, json) = run(&[dir_json.path().to_str().expect("utf-8 path"), "--json"]).await;
    assert_eq!(code, 0, "{json}");
    let seed_json = fs::read_to_string(dir_json.path().join(bench_key::SEED_FILE)).expect("seed");
    let seed_json = seed_json.trim_end();

    for (output, seed) in [(&text, seed_hex), (&json, seed_json)] {
        assert!(!output.contains(seed), "the whole seed is in the output");
        assert!(
            !output.contains(&seed[..8]),
            "the seed's first 8 characters are in the output"
        );
        assert!(
            !output.contains(&seed[seed.len() - 8..]),
            "the seed's last 8 characters are in the output"
        );
    }
}

/// Proof 3: a directory inside a git work tree is refused, naming the repository's top, and nothing
/// is written. A `.git` file (as a linked worktree has) refuses the same way.
#[tokio::test]
async fn a_directory_inside_a_work_tree_is_refused_and_nothing_is_written() {
    let root = fresh();
    fs::create_dir(root.path().join(".git")).expect("a repository marker");
    let inner = root.path().join("a").join("b");
    fs::create_dir_all(&inner).expect("nested directories");

    let (code, out) = run(&[inner.to_str().expect("utf-8 path")]).await;
    assert_eq!(code, 1, "{out}");
    let top = fs::canonicalize(root.path()).expect("canonical");
    assert!(
        out.contains(&top.display().to_string()),
        "the top is named: {out}"
    );
    assert_eq!(
        fs::read_dir(&inner).expect("list").count(),
        0,
        "nothing was written"
    );

    let linked = fresh();
    fs::write(linked.path().join(".git"), "gitdir: /elsewhere\n").expect("a .git file");
    let (code, out) = run(&[linked.path().to_str().expect("utf-8 path")]).await;
    assert_eq!(code, 1, "{out}");
    assert_eq!(
        fs::read_dir(linked.path()).expect("list").count(),
        1,
        "only the .git file"
    );
}

/// Proof 4: an existing `bench.seed`, `bench.pub` or `bench.keys` refuses, naming it; nothing else
/// is written and the existing file is unchanged.
#[tokio::test]
async fn an_existing_key_file_refuses_and_nothing_else_is_written() {
    for name in [
        bench_key::SEED_FILE,
        bench_key::PUBLIC_FILE,
        bench_key::KEYS_FILE,
    ] {
        let dir = fresh();
        let existing = dir.path().join(name);
        fs::write(&existing, "kept\n").expect("the existing file");

        let (code, out) = run(&[dir.path().to_str().expect("utf-8 path")]).await;
        assert_eq!(code, 1, "{name}: {out}");
        assert!(out.contains(name), "{name} is named: {out}");
        assert_eq!(
            fs::read_to_string(&existing).expect("kept"),
            "kept\n",
            "{name} is unchanged"
        );
        assert_eq!(
            fs::read_dir(dir.path()).expect("list").count(),
            1,
            "{name}: nothing else written"
        );
    }
}

/// Proof 5: a directory group or others can write is refused and nothing is written.
#[tokio::test]
async fn a_group_or_other_writable_directory_is_refused() {
    for mode in [0o775, 0o757] {
        let dir = fresh();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(mode)).expect("chmod");

        let (code, out) = run(&[dir.path().to_str().expect("utf-8 path")]).await;
        assert_eq!(code, 1, "{mode:o}: {out}");
        assert!(out.contains("chmod"), "the refusal says what to do: {out}");
        assert_eq!(
            fs::read_dir(dir.path()).expect("list").count(),
            0,
            "{mode:o}: nothing written"
        );

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("chmod back");
    }
}

/// Proof 6: a record signed with the written seed verifies under the public key in `bench.pub`,
/// and not under another key.
#[tokio::test]
async fn a_record_signed_with_the_written_seed_verifies_under_bench_pub() {
    let dir = fresh();
    let (code, out) = run(&[dir.path().to_str().expect("utf-8 path")]).await;
    assert_eq!(code, 0, "{out}");

    let seed = Seed::read(&dir.path().join(bench_key::SEED_FILE)).expect("seed");
    let public_text = fs::read_to_string(dir.path().join(bench_key::PUBLIC_FILE)).expect("pub");
    let public = PublicKey::parse(public_text.trim()).expect("bench.pub parses as a public key");

    let slot_json = br#"{"id":"bench"}"#;
    let record = sign_slot_record(&seed, slot_json, Some(printed_id(&out))).expect("signs");
    assert_eq!(record.len(), 108);
    assert_eq!(
        &record[12..44],
        &hex_decode(printed_id(&out))[..],
        "the record names the key id"
    );
    let signature = &record[44..];
    let message = slot_message(slot_json).expect("message");
    assert!(
        public.verify_bytes(&message, signature),
        "the record verifies under bench.pub"
    );

    let other = Seed::from_bytes([0x09; SEED_LEN]);
    let other_public = other.public_key();
    assert!(
        !other_public.verify_bytes(&message, signature),
        "another key does not verify it"
    );
}

/// The key id printed by a run, as the text the seed check takes.
fn printed_id(out: &str) -> &str {
    line_value(out, "key id: ")
}

/// `--json` gives one document with the key id and the three paths, and the same key id as the
/// seed file reads back to.
#[tokio::test]
async fn the_json_document_holds_the_paths_and_the_key_id() {
    let dir = fresh();
    let (code, out) = run(&[dir.path().to_str().expect("utf-8 path"), "--json"]).await;
    assert_eq!(code, 0, "{out}");
    let document: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON document");
    assert_eq!(document["ok"], true);
    let seed = Seed::read(&dir.path().join(bench_key::SEED_FILE)).expect("seed");
    assert_eq!(
        document["key_id"].as_str(),
        Some(seed.key_id_hex().as_str())
    );
    assert!(
        document["seed_file"]
            .as_str()
            .expect("seed_file")
            .ends_with(bench_key::SEED_FILE)
    );
    assert!(
        document["public_key_file"]
            .as_str()
            .expect("pub")
            .ends_with(bench_key::PUBLIC_FILE)
    );
    assert!(
        document["key_list_file"]
            .as_str()
            .expect("keys")
            .ends_with(bench_key::KEYS_FILE)
    );
}

/// The usage: no directory, or a state option, is a usage error (status 2).
#[tokio::test]
async fn bench_key_usage_errors_are_status_two() {
    let (code, out) = run(&[]).await;
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("bench-key"), "{out}");
    let dir = fresh();
    let (code, _) = run(&[dir.path().to_str().expect("utf-8 path"), "--state", "/x"]).await;
    assert_eq!(code, 2);
}
