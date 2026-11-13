//! The layout checked against the real tool: `veritysetup format --no-superblock`
//! on the same data and salt gives the same root hash and the same tree bytes as
//! this crate.
//!
//! The tool runs when `veritysetup` is on the PATH (CI installs cryptsetup-bin).
//! When it is absent the test prints why and returns; it does not pass by
//! pretending to have checked. The tree is built here the way the kernel's
//! format 1 lays it out, whole in memory, top level first in the file: an
//! independent statement, the same as the bundle's unit test.

use std::fs;
use std::path::Path;
use std::process::Command;

use atlas_bundle::{BLOCK, root_hash};
use atlas_image::tree_bytes;
use sha2::{Digest, Sha256};

/// Digests per hash block.
const FANOUT: usize = BLOCK / 32;

/// Whether the tool runs. Prints the skip reason when it does not.
fn tool_present() -> bool {
    let present = Command::new("veritysetup")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !present {
        eprintln!(
            "SKIPPED veritysetup_cross_check: veritysetup is not on the PATH here; \
             the layout is NOT checked against the real tool in this run (CI installs cryptsetup-bin)"
        );
    }
    present
}

/// A deterministic data region of `blocks` blocks.
fn data_of(blocks: usize) -> Vec<u8> {
    (0..blocks * BLOCK)
        .map(|i| ((i * 31 + i / BLOCK) % 251) as u8)
        .collect()
}

/// The tree of `data` with `salt` in the kernel's format-1 layout, built whole.
/// Levels are built from the data up; the file holds the top level first (one
/// block), then each level down to the data's, every block zero padded.
fn reference_tree(salt: &[u8], data: &[u8]) -> Vec<u8> {
    let digest = |block: &[u8]| -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(block);
        hasher.finalize().into()
    };
    let padded = |digests: &[[u8; 32]]| -> Vec<u8> {
        let mut block: Vec<u8> = digests.iter().flatten().copied().collect();
        block.resize(BLOCK, 0);
        block
    };
    let mut levels: Vec<Vec<[u8; 32]>> = vec![data.chunks(BLOCK).map(digest).collect()];
    while levels.last().is_some_and(|level| level.len() > FANOUT) {
        let below = levels.last().expect("a level");
        let above: Vec<[u8; 32]> = below
            .chunks(FANOUT)
            .map(|chunk| digest(&padded(chunk)))
            .collect();
        levels.push(above);
    }
    let mut tree = Vec::new();
    for level in levels.iter().rev() {
        for chunk in level.chunks(FANOUT) {
            tree.extend(padded(chunk));
        }
    }
    tree
}

/// Runs `veritysetup format` on `data` into `tree` and returns its root hash.
fn veritysetup(data: &Path, tree: &Path, blocks: usize, salt: &[u8]) -> String {
    let mut command = Command::new("veritysetup");
    command
        .arg("format")
        .arg("--no-superblock")
        .arg("--format=1")
        .arg("--hash=sha256")
        .arg("--data-block-size=4096")
        .arg("--hash-block-size=4096")
        .arg(format!("--data-blocks={blocks}"));
    if salt.is_empty() {
        command.arg("--salt-empty");
    } else {
        let hex: String = salt.iter().map(|b| format!("{b:02x}")).collect();
        command.arg(format!("--salt={hex}"));
    }
    let out = command
        .arg(data)
        .arg(tree)
        .output()
        .expect("veritysetup runs");
    assert!(
        out.status.success(),
        "veritysetup failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Root hash:"))
        .map(|hash| hash.trim().to_string())
        .unwrap_or_else(|| panic!("no Root hash line in veritysetup's output:\n{text}"))
}

/// One shape: `blocks` data blocks with `salt`, checked against the tool.
fn check_shape(dir: &Path, blocks: usize, salt: &[u8]) {
    let data = data_of(blocks);
    let data_path = dir.join(format!("data-{blocks}-{}.bin", salt.len()));
    let tree_path = dir.join(format!("tree-{blocks}-{}.bin", salt.len()));
    fs::write(&data_path, &data).expect("write data");
    let _ = fs::remove_file(&tree_path);

    let tool_root = veritysetup(&data_path, &tree_path, blocks, salt);
    let tool_tree = fs::read(&tree_path).expect("the tree file");

    let ours = root_hash(salt, &data).expect("our root");
    let reference = reference_tree(salt, &data);
    assert_eq!(
        tool_root,
        ours,
        "root hash, {blocks} blocks, salt {} bytes",
        salt.len()
    );
    // The root is the digest of the top block, the first block of the reference tree.
    let mut top = Sha256::new();
    top.update(salt);
    top.update(&reference[..BLOCK]);
    let reference_root: String = top.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(reference_root, ours, "the reference's own root");
    assert_eq!(
        tool_tree.len() as u64,
        tree_bytes(blocks as u64),
        "the tool's tree is the length the layout gives"
    );
    assert_eq!(
        tool_tree,
        reference,
        "tree bytes, {blocks} blocks, salt {} bytes",
        salt.len()
    );
    let _ = fs::remove_file(&data_path);
    let _ = fs::remove_file(&tree_path);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("veritysetup-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn the_root_and_tree_match_veritysetup_with_a_salt() {
    if !tool_present() {
        return;
    }
    let dir = scratch("salted");
    let salt = [0x5A_u8; 16];
    // One block; a partial hash block; a block over one hash block (two levels);
    // a count that is no power of the fan-out.
    for blocks in [1, 3, 128, 129, 300] {
        check_shape(&dir, blocks, &salt);
    }
}

#[test]
fn the_root_and_tree_match_veritysetup_with_no_salt() {
    if !tool_present() {
        return;
    }
    let dir = scratch("unsalted");
    for blocks in [1, 129] {
        check_shape(&dir, blocks, &[]);
    }
}
