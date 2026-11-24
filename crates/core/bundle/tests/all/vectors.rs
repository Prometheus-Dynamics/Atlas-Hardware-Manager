//! Lemnos's root-hash vectors (I5-actions.md): `veritysetup` 2.8.7 with
//! `--no-superblock`, sha256, 4096-byte blocks, the salt below, data block `i` filled
//! with the byte `i mod 251`. Each line gives the data block count, the hash tree's
//! bytes and the root hash. No tool is needed: this runs everywhere.
//!
//! The data is streamed block by block into the hasher, one block at a time, so the
//! 38400-block case (about 150 MB) never exists in memory at once.

use atlas_bundle::{BLOCK, VerityHasher};
use atlas_image::tree_bytes;

/// The vectors' salt: 32 bytes, 64 lowercase hex digits.
const SALT: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// (data blocks, hash tree bytes, root hash), exactly as Lemnos's table has them.
const VECTORS: [(u64, u64, &str); 7] = [
    (
        1,
        0,
        "4ce3ecf32c133bf6321901b6092219474b6ac91a19d0304621d629e6bb9987dc",
    ),
    (
        2,
        4096,
        "bd780675f50010ed023fd1f07e98f029a6aaf25a8f6ee33ec94b4b4e847e4b2e",
    ),
    (
        128,
        4096,
        "acb37719c99f3ac554ecbbccb8910616915a08e5a67bd3b9778694c60e673da4",
    ),
    (
        129,
        12288,
        "6093a2333523050b628581510028976d3d3e9c62458642a83727e6df641397a4",
    ),
    (
        16384,
        528384,
        "234a6801a6840c2defdf3a7c08060cf9874f5773c6360f4172b08ae891a611d5",
    ),
    (
        16385,
        540672,
        "f1acccee31dcae56f612342ed838ba4c1f375de2d0341ae0cddecd0b28ff8732",
    ),
    (
        38400,
        1245184,
        "a73f8f9623fc3e3298871b6ef9de8ae4ba8867f23f5d8a92b87d97b0b5ddfa18",
    ),
];

/// The root of `blocks` data blocks, streamed one block at a time.
fn streamed_root(blocks: u64) -> String {
    let mut hasher = VerityHasher::new(&SALT);
    let mut block = vec![0u8; BLOCK];
    for i in 0..blocks {
        block.fill((i % 251) as u8);
        hasher.update(&block);
    }
    hasher.finish().expect("a whole number of blocks")
}

#[test]
fn lemnos_vectors_hold_for_the_tree_bytes_and_the_root() {
    for (blocks, tree, root) in VECTORS {
        assert_eq!(tree_bytes(blocks), tree, "tree bytes, {blocks} data blocks");
        assert_eq!(
            streamed_root(blocks),
            root,
            "root hash, {blocks} data blocks"
        );
    }
}
