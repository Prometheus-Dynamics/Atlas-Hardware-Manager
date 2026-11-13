//! The dm-verity values a slot's manifest carries, and the layout they must describe.
//!
//! A slot's manifest gives four values for its hash tree: `verity.root_hash`,
//! `verity.salt`, `verity.data_blocks` and `verity.hash_offset`. Everything else about
//! the tree is fixed by convention and is not in the manifest:
//!
//! - dm-verity format 1, with no verity superblock ([`FORMAT`]);
//! - SHA-256 digests ([`ALGORITHM`]), 4096-byte data blocks and 4096-byte hash blocks
//!   ([`BLOCK`]);
//! - the hash tree appended directly after the data in the slot file: the file is the
//!   `data_blocks` data blocks, then the tree, which starts at byte `hash_offset`.
//!
//! The salt is prepended to every block before it is hashed (format 1): a block's
//! digest is `sha256(salt || block)`. A hash block holds 128 digests, zero padded. The
//! root hash is the digest of the single top hash block.
//!
//! What is trusted at boot is the root hash, and only it: it is on the kernel command
//! line, so it is as trusted as the kernel and initramfs beside it. A signature on the
//! slot's manifest (`slot.sig`) protects what is INSTALLED, not what boots; nothing
//! here calls a signed slot "verified boot".

use thiserror::Error;

/// The dm-verity format, the kernel's `verity 1` table line.
pub const FORMAT: u32 = 1;
/// The hash algorithm of the tree, by its kernel name.
pub const ALGORITHM: &str = "sha256";
/// The data block and hash block size, in bytes, for both the data and the tree.
pub const BLOCK: u64 = 4096;
/// The most hex digits a manifest's salt may carry (256 bytes).
pub const MAX_SALT_HEX: usize = 512;

/// Bytes in one digest.
const DIGEST: u64 = 32;
/// Digests in one hash block.
const FANOUT: u64 = BLOCK / DIGEST;

/// The four verity values of one slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verity {
    /// The root hash, lowercase hex, 64 digits.
    pub root_hash: String,
    /// The salt's bytes; empty for no salt.
    pub salt: Vec<u8>,
    /// How many 4096-byte data blocks the tree covers.
    pub data_blocks: u64,
    /// The byte offset of the tree in the slot file: `data_blocks * BLOCK`.
    pub hash_offset: u64,
}

/// Why a slot's verity values do not describe a slot file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerityError {
    #[error("verity.root_hash {0:?} is not 64 lowercase hex digits")]
    RootHash(String),
    #[error("verity.salt {0:?} is not lowercase hex of an even length up to 512 digits")]
    Salt(String),
    #[error("verity.data_blocks is {0}: the data is at least one 4096-byte block")]
    DataBlocks(u64),
    #[error(
        "verity.hash_offset is {hash_offset}, but verity.data_blocks {data_blocks} times 4096 is {expected}: the tree follows the data"
    )]
    Offset {
        data_blocks: u64,
        hash_offset: u64,
        expected: u64,
    },
    #[error(
        "the slot file is {size} bytes, but its {data_blocks} data blocks and their hash tree make {expected}: the file is cut short, or its tree or verity.data_blocks is wrong"
    )]
    Size {
        data_blocks: u64,
        size: u64,
        expected: u64,
    },
}

impl Verity {
    /// Makes the values from a manifest's fields, checking each: the root hash and
    /// salt are lowercase hex, the data is at least one block, and the tree starts
    /// where the data ends. The slot file's length is checked by [`Verity::check_size`].
    ///
    /// # Errors
    ///
    /// The first value that does not fit, named by its own [`VerityError`].
    pub fn from_manifest(
        root_hash: &str,
        salt: &str,
        data_blocks: u64,
        hash_offset: u64,
    ) -> Result<Self, VerityError> {
        if !is_hex(salt, salt.len()) || !salt.len().is_multiple_of(2) || salt.len() > MAX_SALT_HEX {
            return Err(VerityError::Salt(salt.to_string()));
        }
        let salt = (0..salt.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&salt[i..i + 2], 16).unwrap_or_default())
            .collect();
        let verity = Self {
            root_hash: root_hash.to_string(),
            salt,
            data_blocks,
            hash_offset,
        };
        verity.check()?;
        Ok(verity)
    }

    /// Checks the values against the layout: the root hash is 64 lowercase hex digits,
    /// the data is at least one block, and `hash_offset` is `data_blocks * BLOCK`.
    ///
    /// # Errors
    ///
    /// The first value that does not fit.
    pub fn check(&self) -> Result<(), VerityError> {
        if !is_hex(&self.root_hash, 64) {
            return Err(VerityError::RootHash(self.root_hash.clone()));
        }
        if self.data_blocks == 0 || self.data_blocks.checked_mul(BLOCK).is_none() {
            return Err(VerityError::DataBlocks(self.data_blocks));
        }
        let expected = self.data_len();
        if self.hash_offset != expected {
            return Err(VerityError::Offset {
                data_blocks: self.data_blocks,
                hash_offset: self.hash_offset,
                expected,
            });
        }
        Ok(())
    }

    /// Checks that a slot file of `size` bytes is its data and its whole tree:
    /// `size` is `hash_offset` plus [`tree_bytes`] of `data_blocks`.
    ///
    /// # Errors
    ///
    /// [`VerityError::Size`], with the expected length.
    pub fn check_size(&self, size: u64) -> Result<(), VerityError> {
        let expected = self.data_len().saturating_add(tree_bytes(self.data_blocks));
        if size != expected {
            return Err(VerityError::Size {
                data_blocks: self.data_blocks,
                size,
                expected,
            });
        }
        Ok(())
    }

    /// The data's length in bytes: `data_blocks * BLOCK`, saturating.
    pub fn data_len(&self) -> u64 {
        self.data_blocks.saturating_mul(BLOCK)
    }

    /// The salt as lowercase hex; empty when there is no salt.
    pub fn salt_hex(&self) -> String {
        self.salt.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The size of the hash tree over `data_blocks` data blocks, in bytes.
///
/// Level 0 has one digest per data block, 128 to a hash block: `ceil(data_blocks /
/// 128)` blocks. Each level above has one digest per block of the level below, so
/// level n+1 is `ceil(level n / 128)` blocks. The levels stop at the first one that is a
/// single block, the root's. The tree is the sum of the levels, times 4096.
///
/// Checked values: 1 block gives 1 block (4096); 128 gives 1 (4096); 129 gives 2 + 1
/// (12288); 16384 gives 128 + 1 (129 blocks); 16385 gives 129 + 2 + 1 (132 blocks).
pub fn tree_bytes(data_blocks: u64) -> u64 {
    let mut blocks = 0u64;
    let mut level = data_blocks;
    loop {
        level = level.div_ceil(FANOUT);
        blocks = blocks.saturating_add(level);
        if level <= 1 {
            return blocks.saturating_mul(BLOCK);
        }
    }
}

/// `len` lowercase hex digits (`len` 0 is the empty string).
fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "ab00000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn the_tree_size_follows_the_fan_out_at_every_level() {
        assert_eq!(tree_bytes(1), BLOCK);
        assert_eq!(tree_bytes(4), BLOCK);
        assert_eq!(tree_bytes(128), BLOCK);
        // One level over the root: 129 digests need two hash blocks, then the root.
        assert_eq!(tree_bytes(129), 3 * BLOCK);
        assert_eq!(tree_bytes(256), 3 * BLOCK);
        // Two levels exactly: 16384 digests, 128 blocks, then the root.
        assert_eq!(tree_bytes(16_384), 129 * BLOCK);
        // A count that is not a power of the fan-out: 129 + 2 + 1 blocks.
        assert_eq!(tree_bytes(16_385), 132 * BLOCK);
    }

    #[test]
    fn values_that_describe_a_slot_are_taken() {
        let v = Verity::from_manifest(ROOT, "5a5a", 4, 4 * BLOCK).expect("fits");
        assert_eq!(v.salt, vec![0x5a, 0x5a]);
        assert_eq!(v.salt_hex(), "5a5a");
        assert_eq!(v.data_len(), 4 * BLOCK);
        v.check_size(4 * BLOCK + BLOCK).expect("one tree block");
        let unsalted = Verity::from_manifest(ROOT, "", 1, BLOCK).expect("no salt");
        assert!(unsalted.salt.is_empty());
    }

    #[test]
    fn a_salt_that_is_not_lowercase_even_hex_is_refused() {
        for bad in ["abc", "zz", "5A5A", "5a 5a", &"a".repeat(514)] {
            assert_eq!(
                Verity::from_manifest(ROOT, bad, 1, BLOCK),
                Err(VerityError::Salt(bad.to_string())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_root_that_is_not_a_digest_is_refused() {
        assert!(matches!(
            Verity::from_manifest("abc", "", 1, BLOCK),
            Err(VerityError::RootHash(_))
        ));
    }

    #[test]
    fn a_block_count_of_zero_or_one_that_overflows_is_refused() {
        assert_eq!(
            Verity::from_manifest(ROOT, "", 0, 0),
            Err(VerityError::DataBlocks(0))
        );
        assert_eq!(
            Verity::from_manifest(ROOT, "", u64::MAX, 0),
            Err(VerityError::DataBlocks(u64::MAX))
        );
    }

    #[test]
    fn an_offset_that_is_not_the_end_of_the_data_is_refused() {
        assert_eq!(
            Verity::from_manifest(ROOT, "", 4, 3 * BLOCK),
            Err(VerityError::Offset {
                data_blocks: 4,
                hash_offset: 3 * BLOCK,
                expected: 4 * BLOCK,
            })
        );
    }

    #[test]
    fn a_file_without_its_whole_tree_is_refused_with_the_expected_length() {
        let v = Verity::from_manifest(ROOT, "", 4, 4 * BLOCK).expect("fits");
        assert_eq!(
            v.check_size(4 * BLOCK),
            Err(VerityError::Size {
                data_blocks: 4,
                size: 4 * BLOCK,
                expected: 5 * BLOCK,
            })
        );
        // A tree one block longer than the formula is refused too.
        assert!(v.check_size(6 * BLOCK).is_err());
        // A count that is one too small, with the offset made to fit, is refused by size.
        let fewer = Verity::from_manifest(ROOT, "", 3, 3 * BLOCK).expect("fits");
        assert!(fewer.check_size(5 * BLOCK).is_err());
    }
}
