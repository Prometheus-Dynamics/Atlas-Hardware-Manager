//! The dm-verity root hash, streamed, and Gaia's verity table file.
//!
//! The layout is the kernel's format 1 with SHA-256, 4 KiB data and hash
//! blocks, and a salt that is prepended to every hashed block: a block's
//! digest is `sha256(salt || block)`; a hash block holds 128 digests, zero
//! padded; the root is the digest of the single top hash block. Gaia's
//! `gaia.verity` makes the tree with `veritysetup` in this layout.
//!
//! [`VerityHasher`] keeps one partial block per tree level, so its memory is
//! a few 4 KiB blocks whatever the image's size.

use sha2::{Digest, Sha256};

use crate::BundleError;
use crate::manifest::is_hex;

/// The data and hash block size.
pub const BLOCK: usize = 4096;
/// Digests per hash block.
const DIGESTS_PER_BLOCK: usize = BLOCK / 32;

/// The fields of Gaia's `verity.table` that the writer needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerityTable {
    /// Lowercase hex.
    pub root_hash: String,
    pub salt: Vec<u8>,
    /// The data's length in blocks; the data is `data_blocks * BLOCK` bytes.
    pub data_blocks: u64,
    /// Where the tree starts. Gaia puts it right after the padded data.
    pub hash_offset: u64,
}

impl VerityTable {
    /// Reads Gaia's table text: one `key value` line per field, the dm-verity
    /// table line last. Unknown keys are ignored; a table that is not the
    /// kernel format, sha256 with 4 KiB blocks, is refused.
    pub fn parse(text: &str) -> Result<Self, BundleError> {
        let field = |key: &str| {
            text.lines().find_map(|line| {
                let (name, value) = line.split_once(' ')?;
                (name == key).then_some(value.trim())
            })
        };
        let need =
            |key: &str| field(key).ok_or_else(|| BundleError::Table(format!("no {key} line")));
        let refuse = |why: String| BundleError::Table(why);

        if need("hash_type")? != "1" {
            return Err(refuse("hash_type is not 1 (the kernel format)".into()));
        }
        if need("algorithm")? != "sha256" {
            return Err(refuse("algorithm is not sha256".into()));
        }
        for key in ["data_block_size", "hash_block_size"] {
            if need(key)? != BLOCK.to_string() {
                return Err(refuse(format!("{key} is not {BLOCK}")));
            }
        }
        let root_hash = need("root_hash")?.to_ascii_lowercase();
        if !is_hex(&root_hash, 64) {
            return Err(refuse("root_hash is not a SHA-256 digest".into()));
        }
        let salt = need("salt")?;
        let salt = decode_hex(salt).ok_or_else(|| refuse("salt is not hex".into()))?;
        let data_blocks: u64 = need("data_blocks")?
            .parse()
            .map_err(|_| refuse("data_blocks is not a number".into()))?;
        let hash_offset: u64 = need("hash_offset")?
            .parse()
            .map_err(|_| refuse("hash_offset is not a number".into()))?;
        if data_blocks == 0 {
            return Err(refuse("data_blocks is 0".into()));
        }
        let table = Self {
            root_hash,
            salt,
            data_blocks,
            hash_offset,
        };
        // Gaia puts the tree right after the padded data, so the data is
        // exactly `hash_offset` bytes long and the tree follows it.
        if table.data_len().map(|len| len == hash_offset) != Some(true) {
            return Err(refuse(
                "hash_offset is not the end of the padded data".into(),
            ));
        }
        Ok(table)
    }

    /// The data's length in bytes, or `None` if it does not fit in a `u64`.
    pub fn data_len(&self) -> Option<u64> {
        self.data_blocks.checked_mul(BLOCK as u64)
    }
}

/// The root hash of `data` (a whole number of blocks) with `salt`, in one
/// call. For the streamed form use [`VerityHasher`].
pub fn root_hash(salt: &[u8], data: &[u8]) -> Result<String, BundleError> {
    let mut hasher = VerityHasher::new(salt);
    hasher.update(data);
    hasher.finish()
}

/// Streams data blocks into a dm-verity root hash. The data is hashed as it
/// is given, in any chunks; the input must end on a block boundary.
#[derive(Debug)]
pub struct VerityHasher {
    salt: Vec<u8>,
    /// The data block being filled, up to `BLOCK` bytes.
    block: Vec<u8>,
    /// One entry per tree level, bottom first.
    levels: Vec<Level>,
}

/// A tree level: the digests waiting for their hash block, and how many
/// hash blocks of the level were already written out.
#[derive(Debug, Default)]
struct Level {
    pending: Vec<u8>,
    flushed: u64,
}

impl VerityHasher {
    pub fn new(salt: &[u8]) -> Self {
        Self {
            salt: salt.to_vec(),
            block: Vec::with_capacity(BLOCK),
            levels: Vec::new(),
        }
    }

    /// Hashes the next bytes of the data.
    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let room = BLOCK - self.block.len();
            let take = room.min(data.len());
            self.block.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.block.len() == BLOCK {
                let block = std::mem::take(&mut self.block);
                let digest = self.digest(&block);
                self.block = Vec::with_capacity(BLOCK);
                self.push(0, digest);
            }
        }
    }

    /// The root hash, lowercase hex. Fails if the data was empty or did not
    /// end on a block boundary.
    pub fn finish(mut self) -> Result<String, BundleError> {
        if !self.block.is_empty() {
            return Err(BundleError::Table(
                "the data does not end on a 4 KiB block".into(),
            ));
        }
        if self.levels.is_empty() {
            return Err(BundleError::Table("the data is empty".into()));
        }
        let mut level = 0;
        loop {
            let Some(current) = self.levels.get_mut(level) else {
                return Err(BundleError::Table("the hash tree has no top".into()));
            };
            let pending = std::mem::take(&mut current.pending);
            if current.flushed == 0 {
                // Nothing was written out at this level, so its pending
                // digests are the whole level: one block, the top.
                let mut block = pending;
                block.resize(BLOCK, 0);
                return Ok(to_hex(&self.digest(&block)));
            }
            if !pending.is_empty() {
                let mut block = pending;
                block.resize(BLOCK, 0);
                current.flushed += 1;
                let digest = self.digest(&block);
                self.push(level + 1, digest);
            }
            level += 1;
        }
    }

    /// Adds a digest to `level`. A full hash block is written out only when
    /// another digest arrives for it, so the last block of a level is never
    /// written early: a level of up to 128 digests is one block, the top.
    fn push(&mut self, level: usize, digest: [u8; 32]) {
        if self.levels.len() <= level {
            self.levels.push(Level::default());
        }
        if self.levels[level].pending.len() == DIGESTS_PER_BLOCK * 32 {
            let block = std::mem::take(&mut self.levels[level].pending);
            self.levels[level].flushed += 1;
            let up = self.digest(&block);
            self.push(level + 1, up);
        }
        self.levels[level].pending.extend_from_slice(&digest);
    }

    fn digest(&self, block: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.salt);
        hasher.update(block);
        hasher.finalize().into()
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.len().is_multiple_of(2) || text.len() > 512 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree built whole in memory, level by level: an independent
    /// statement of the same layout.
    fn reference(salt: &[u8], data: &[u8]) -> String {
        let digest = |block: &[u8]| -> [u8; 32] {
            let mut hasher = Sha256::new();
            hasher.update(salt);
            hasher.update(block);
            hasher.finalize().into()
        };
        let mut level: Vec<[u8; 32]> = data.chunks(BLOCK).map(digest).collect();
        loop {
            if level.len() <= DIGESTS_PER_BLOCK {
                let mut block = Vec::new();
                for d in &level {
                    block.extend_from_slice(d);
                }
                block.resize(BLOCK, 0);
                return to_hex(&digest(&block));
            }
            level = level
                .chunks(DIGESTS_PER_BLOCK)
                .map(|chunk| {
                    let mut block = Vec::new();
                    for d in chunk {
                        block.extend_from_slice(d);
                    }
                    block.resize(BLOCK, 0);
                    digest(&block)
                })
                .collect();
        }
    }

    fn blocks(n: usize) -> Vec<u8> {
        (0..n * BLOCK)
            .map(|i| ((i * 31 + i / BLOCK) % 251) as u8)
            .collect()
    }

    #[test]
    fn the_streamed_root_is_the_tree_built_whole() {
        // One block, a full hash block, one block over it, two levels, three.
        for n in [1, 2, 127, 128, 129, 256, 257, 16384, 16385] {
            let data = blocks(n);
            let salt = b"\x01\x02salt";
            assert_eq!(
                root_hash(salt, &data).expect("root"),
                reference(salt, &data),
                "{n} blocks"
            );
        }
    }

    #[test]
    fn chunking_does_not_change_the_root() {
        let data = blocks(300);
        let whole = root_hash(b"s", &data).expect("whole");
        let mut hasher = VerityHasher::new(b"s");
        for chunk in data.chunks(1000) {
            hasher.update(chunk);
        }
        assert_eq!(hasher.finish().expect("chunked"), whole);
    }

    #[test]
    fn data_must_be_whole_blocks_and_not_empty() {
        let mut partial = VerityHasher::new(b"s");
        partial.update(&[0u8; 10]);
        assert!(partial.finish().is_err());
        assert!(VerityHasher::new(b"s").finish().is_err());
    }

    #[test]
    fn gaias_table_text_is_read() {
        // The layout Gaia's `verity.table` writes (gaia.verity).
        let text = format!(
            "hash_type 1\nalgorithm sha256\nroot_hash {}\nsalt {}\nuuid 01234567-89ab-4cde-8f01-23456789abcd\n\
             data_blocks 3\ndata_block_size 4096\nhash_block_size 4096\n\
             hash_offset 12288\nhash_start_block 4\ntable 0 24 verity 1 /dev/vda3 /dev/vda3 4096 4096 3 4 sha256 x y\n",
            "AB".repeat(32),
            "cd".repeat(32)
        );
        let table = VerityTable::parse(&text).expect("parses");
        assert_eq!(table.root_hash, "ab".repeat(32));
        assert_eq!(table.salt, vec![0xcd; 32]);
        assert_eq!(table.data_blocks, 3);
        assert_eq!(table.data_len(), Some(12288));
    }

    #[test]
    fn a_table_that_is_not_the_kernel_format_is_refused() {
        let good = |edit: &str| {
            let base = format!(
                "hash_type 1\nalgorithm sha256\nroot_hash {}\nsalt abcd\ndata_blocks 3\n\
                 data_block_size 4096\nhash_block_size 4096\nhash_offset 12288\n",
                "ab".repeat(32)
            );
            base.replace(edit, "")
        };
        assert!(VerityTable::parse(&good("")).is_ok());
        for bad in [
            "hash_type 1\n",
            "algorithm sha256\n",
            "data_block_size 4096\n",
            "salt abcd\n",
            "hash_offset 12288\n",
        ] {
            assert!(VerityTable::parse(&good(bad)).is_err(), "without {bad:?}");
        }
        let wrong_offset = good("hash_offset 12288\n") + "hash_offset 4096\n";
        assert!(VerityTable::parse(&wrong_offset).is_err());
        let wrong_block = good("data_block_size 4096\n") + "data_block_size 512\n";
        assert!(VerityTable::parse(&wrong_block).is_err());
    }
}
