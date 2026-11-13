//! The layout checked against the real tool: `veritysetup format --no-superblock`
//! on the same data and salt gives the same root hash and the same tree bytes as
//! this crate.
//!
//! The tool runs when `veritysetup` is on the PATH (CI installs cryptsetup-bin).
//! When it is absent the test prints why and returns: it does not pass by pretending
//! to have checked. The tree is built here the way the kernel's format 1 lays it out,
//! whole in memory, top level first in the file: an independent statement.
//!
//! Data: block `i` is filled with the byte `i mod 251`, so Lemnos's root-hash vectors
//! (measured on the same data) can be added to these tests verbatim when they are
//! published in I5-actions.md. The salt is always 32 bytes, as a manifest requires.

use std::fs;
use std::path::Path;
use std::process::Command;

use atlas_bundle::{BLOCK, root_hash};
use atlas_image::tree_bytes;
use sha2::{Digest, Sha256};

/// Digests per hash block.
const FANOUT: usize = BLOCK / 32;

/// A salt for the tests: 32 bytes, not a repeated byte, so a byte-order slip shows.
fn salt() -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11))
}

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

/// `blocks` data blocks; block `i` is all bytes `i mod 251`.
fn data_of(blocks: usize) -> Vec<u8> {
    (0..blocks)
        .flat_map(|i| std::iter::repeat_n((i % 251) as u8, BLOCK))
        .collect()
}

/// The tree of `data` with `salt` in the kernel's format-1 layout, built whole. One
/// data block has no tree. Otherwise the levels are built from the data up, and the
/// file holds the top level first (one block), then each level down to the data's,
/// every block zero padded.
fn reference_tree(salt: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() == BLOCK {
        return Vec::new();
    }
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

/// Runs `veritysetup format` on `data` into `tree`, with the 32-byte salt as 64 hex
/// digits. Returns the root hash the tool printed and its whole output.
fn veritysetup(data: &Path, tree: &Path, blocks: usize, salt: &[u8]) -> (String, String) {
    let hex: String = salt.iter().map(|b| format!("{b:02x}")).collect();
    let out = Command::new("veritysetup")
        .arg("format")
        .arg("--no-superblock")
        .arg("--format=1")
        .arg("--hash=sha256")
        .arg("--data-block-size=4096")
        .arg("--hash-block-size=4096")
        .arg(format!("--data-blocks={blocks}"))
        .arg(format!("--salt={hex}"))
        .arg(data)
        .arg(tree)
        .output()
        .expect("veritysetup runs");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "veritysetup failed:\n{text}");
    let root = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Root hash:"))
        .map(|hash| hash.trim().to_string())
        .unwrap_or_else(|| panic!("no Root hash line in veritysetup's output:\n{text}"));
    (root, text)
}

/// One shape: `blocks` data blocks with the test salt, checked against the tool.
fn check_shape(dir: &Path, blocks: usize) {
    let salt = salt();
    let data = data_of(blocks);
    let data_path = dir.join(format!("data-{blocks}.bin"));
    let tree_path = dir.join(format!("tree-{blocks}.bin"));
    fs::write(&data_path, &data).expect("write data");
    let _ = fs::remove_file(&tree_path);

    let (tool_root, tool_output) = veritysetup(&data_path, &tree_path, blocks, &salt);
    // A tree file the tool did not write is an empty one, for one block.
    let tool_tree = fs::read(&tree_path).unwrap_or_default();
    let context = format!(
        "veritysetup said:\n{tool_output}\ntree file: {} bytes, data file: {} bytes",
        tool_tree.len(),
        data.len()
    );

    let ours = root_hash(&salt, &data).expect("our root");
    let reference = reference_tree(&salt, &data);
    // The tree first: if it differs, the layout differs, and the diff says where.
    assert_eq!(
        tool_tree.len() as u64,
        tree_bytes(blocks as u64),
        "the tool's hash file is the length the layout gives, {blocks} blocks\n{context}"
    );
    assert_eq!(
        tool_tree, reference,
        "tree bytes, {blocks} blocks\n{context}"
    );
    // The tool's root is the salted digest of its top block; for one block the top
    // block is the data block itself, since there is no tree.
    let tool_top: &[u8] = if tool_tree.is_empty() {
        &data
    } else {
        &tool_tree[..BLOCK]
    };
    let mut top = Sha256::new();
    top.update(salt);
    top.update(tool_top);
    let tool_top_root: String = top.finalize().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        tool_root, tool_top_root,
        "the tool's root is salt || its top block, {blocks} blocks\n{context}"
    );
    assert_eq!(tool_root, ours, "root hash, {blocks} blocks\n{context}");
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
fn the_root_and_tree_match_veritysetup_with_a_32_byte_salt() {
    if !tool_present() {
        return;
    }
    let dir = scratch("salted");
    // One block (no tree); two blocks (one hash block); a full hash block; one block
    // over it (two levels); a few hundred blocks.
    for blocks in [1, 2, 128, 129, 300] {
        check_shape(&dir, blocks);
    }
}
