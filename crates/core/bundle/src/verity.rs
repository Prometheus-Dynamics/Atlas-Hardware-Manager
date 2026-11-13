//! The dm-verity root hash of a slot's data, streamed.
//!
//! The layout is the one `atlas_image::verity` states: the kernel's format 1,
//! SHA-256, 4 KiB data and hash blocks. Format 1 "prepends the salt when hashing"
//! (the kernel's `Documentation/admin-guide/device-mapper/verity.rst`, version 1),
//! so a block's digest is `sha256(salt || block)`; each digest is padded to a
//! power of two, which a 32-byte SHA-256 digest already is. A hash block holds 128
//! digests, zero padded; the root is the digest of the single top hash block,
//! with the salt before it. Format 0, which appends the salt, is not used. The
//! `veritysetup` test in `tests/veritysetup.rs` checks this against the real tool.
//!
//! [`VerityHasher`] keeps one partial block per tree level, so its memory is a few
//! 4 KiB blocks whatever the image's size. It recomputes the ROOT only. The tree
//! bytes appended to the data are not recomputed (see the crate docs).

use sha2::{Digest, Sha256};

use crate::BundleError;

/// The data and hash block size, in bytes, as the `usize` the buffers want.
pub const BLOCK: usize = atlas_image::BLOCK as usize;
/// Digests per hash block.
const DIGESTS_PER_BLOCK: usize = BLOCK / 32;

/// The root hash of `data` (a whole number of blocks) with `salt`, in one call.
/// For the streamed form use [`VerityHasher`].
pub fn root_hash(salt: &[u8], data: &[u8]) -> Result<String, BundleError> {
    let mut hasher = VerityHasher::new(salt);
    hasher.update(data);
    hasher.finish()
}

/// Streams data blocks into a dm-verity root hash. The data is hashed as it is
/// given, in any chunks; the input must end on a block boundary.
#[derive(Debug)]
pub struct VerityHasher {
    salt: Vec<u8>,
    /// The data block being filled, up to `BLOCK` bytes.
    block: Vec<u8>,
    /// One entry per tree level, bottom first.
    levels: Vec<Level>,
}

/// A tree level: the digests waiting for their hash block, and how many hash
/// blocks of the level were already written out.
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

    /// The root hash, lowercase hex. Fails if the data was empty or did not end
    /// on a block boundary.
    pub fn finish(mut self) -> Result<String, BundleError> {
        if !self.block.is_empty() {
            return Err(BundleError::Malformed(
                "the verity data does not end on a 4 KiB block".into(),
            ));
        }
        if self.levels.is_empty() {
            return Err(BundleError::Malformed("the verity data is empty".into()));
        }
        let mut level = 0;
        loop {
            let Some(current) = self.levels.get_mut(level) else {
                return Err(BundleError::Malformed("the hash tree has no top".into()));
            };
            let pending = std::mem::take(&mut current.pending);
            if current.flushed == 0 {
                // Nothing was written out at this level, so its pending digests are
                // the whole level: one block, the top.
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

    /// Adds a digest to `level`. A full hash block is written out only when another
    /// digest arrives for it, so the last block of a level is never written early: a
    /// level of up to 128 digests is one block, the top.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree built whole in memory, level by level: an independent statement of
    /// the same layout.
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
    fn the_salt_is_prepended_to_every_block() {
        // A block hashed with the salt is not the block hashed alone.
        let data = blocks(1);
        assert_ne!(
            root_hash(b"salt", &data).expect("salted"),
            root_hash(b"", &data).expect("unsalted")
        );
    }
}
