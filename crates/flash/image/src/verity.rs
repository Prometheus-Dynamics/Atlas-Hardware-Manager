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
//! The salt is REQUIRED: exactly 32 bytes, written as 64 lowercase hex digits, random
//! per image ([`SALT_LEN`]). There is no empty-salt case. The salt is prepended to every
//! block before it is hashed (format 1): a block's digest is `sha256(salt || block)`. A
//! hash block holds 128 digests, zero padded. The root hash is the digest of the single
//! top hash block, or, for one data block (which has no tree), the digest of that block.
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
/// The salt's length in bytes. A manifest's salt is these bytes as 64 hex digits.
pub const SALT_LEN: usize = 32;

/// Bytes in one digest.
const DIGEST: u64 = 32;
/// Digests in one hash block.
const FANOUT: u64 = BLOCK / DIGEST;

/// The four verity values of one slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verity {
    /// The root hash, lowercase hex, 64 digits.
    pub root_hash: String,
    /// The salt: exactly [`SALT_LEN`] bytes.
    pub salt: [u8; SALT_LEN],
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
    #[error("verity.salt is missing: a slot needs its salt (64 hex digits, 32 random bytes)")]
    SaltMissing,
    #[error("verity.salt is empty: there is no empty-salt case; it must be 64 hex digits")]
    SaltEmpty,
    #[error(
        "verity.salt is {digits} hex digits: it must be exactly 64 (32 bytes), random per image"
    )]
    SaltLength { digits: usize },
    #[error("verity.salt {0:?} is not 64 lowercase hex digits")]
    SaltNotHex(String),
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
    /// Makes the values from a manifest's fields, checking each. The salt must be
    /// present, not empty, exactly 64 lowercase hex digits; the root hash is 64
    /// lowercase hex digits; the data is at least one block; and the tree starts where
    /// the data ends. The slot file's length is checked by [`Verity::check_size`].
    ///
    /// # Errors
    ///
    /// The first value that does not fit, named by its own [`VerityError`].
    pub fn from_manifest(
        root_hash: &str,
        salt: Option<&str>,
        data_blocks: u64,
        hash_offset: u64,
    ) -> Result<Self, VerityError> {
        let salt = parse_salt(salt)?;
        let verity = Self {
            root_hash: root_hash.to_string(),
            salt,
            data_blocks,
            hash_offset,
        };
        verity.check()?;
        Ok(verity)
    }

    /// Checks the layout: the root hash is 64 lowercase hex digits, the data is at least
    /// one block, and `hash_offset` is `data_blocks * BLOCK`. The salt is a fixed-size
    /// array, so its length is already right.
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

    /// The salt as 64 lowercase hex digits.
    pub fn salt_hex(&self) -> String {
        self.salt.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Reads a manifest's salt: present, 64 lowercase hex digits, 32 bytes.
fn parse_salt(salt: Option<&str>) -> Result<[u8; SALT_LEN], VerityError> {
    let salt = salt.ok_or(VerityError::SaltMissing)?;
    if salt.is_empty() {
        return Err(VerityError::SaltEmpty);
    }
    if salt.len() != SALT_LEN * 2 {
        return Err(VerityError::SaltLength { digits: salt.len() });
    }
    if !is_hex(salt, SALT_LEN * 2) {
        return Err(VerityError::SaltNotHex(salt.to_string()));
    }
    let mut bytes = [0u8; SALT_LEN];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&salt[2 * i..2 * i + 2], 16).unwrap_or_default();
    }
    Ok(bytes)
}

/// The size of the hash tree over `data_blocks` data blocks, in bytes.
///
/// One data block has no levels: its root is its own salted digest, and the tree is
/// empty (veritysetup writes "Hash blocks: 0"). Otherwise: each level is `ceil(n / 128)`
/// blocks of the count below it (128 digests to a hash block), repeated WHILE the count
/// being hashed is greater than one; the tree is the sum of the levels, times 4096.
///
/// The measured values (veritysetup 2.8.7, data blocks to hash file bytes): 1 to 0;
/// 2 to 4096; 128 to 4096; 129 to 12288; 16384 to 528384; 16385 to 540672; 38400 to
/// 1245184. The unit test below checks each of them.
pub fn tree_bytes(data_blocks: u64) -> u64 {
    if data_blocks <= 1 {
        return 0;
    }
    let mut blocks = 0u64;
    let mut count = data_blocks;
    while count > 1 {
        count = count.div_ceil(FANOUT);
        blocks = blocks.saturating_add(count);
    }
    blocks.saturating_mul(BLOCK)
}

/// `len` lowercase hex digits (`len` 0 is the empty string).
fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "ab00000000000000000000000000000000000000000000000000000000000000";
    const SALT: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

    #[test]
    fn the_tree_size_matches_the_measured_veritysetup_values() {
        // data blocks to hash file bytes, measured by veritysetup 2.8.7 (Lemnos).
        let measured = [
            (1, 0),
            (2, 4096),
            (128, 4096),
            (129, 12_288),
            (16_384, 528_384),
            (16_385, 540_672),
            (38_400, 1_245_184),
        ];
        for (blocks, bytes) in measured {
            assert_eq!(tree_bytes(blocks), bytes, "{blocks} data blocks");
        }
    }

    #[test]
    fn values_that_describe_a_slot_are_taken() {
        let v = Verity::from_manifest(ROOT, Some(SALT), 4, 4 * BLOCK).expect("fits");
        assert_eq!(v.salt, [0x5a; SALT_LEN]);
        assert_eq!(v.salt_hex(), SALT);
        assert_eq!(v.data_len(), 4 * BLOCK);
        v.check_size(4 * BLOCK + BLOCK).expect("one tree block");
        // One data block has no tree: its whole length is its data.
        let one = Verity::from_manifest(ROOT, Some(SALT), 1, BLOCK).expect("one block");
        one.check_size(BLOCK).expect("no tree");
    }

    #[test]
    fn a_missing_empty_short_long_or_non_hex_salt_each_has_its_own_error() {
        assert_eq!(
            Verity::from_manifest(ROOT, None, 1, BLOCK),
            Err(VerityError::SaltMissing)
        );
        assert_eq!(
            Verity::from_manifest(ROOT, Some(""), 1, BLOCK),
            Err(VerityError::SaltEmpty)
        );
        assert_eq!(
            Verity::from_manifest(ROOT, Some("5a5a"), 1, BLOCK),
            Err(VerityError::SaltLength { digits: 4 })
        );
        // 32 digits (16 bytes) and an odd length are refused as the wrong length.
        let half = &SALT[..32];
        assert_eq!(
            Verity::from_manifest(ROOT, Some(half), 1, BLOCK),
            Err(VerityError::SaltLength { digits: 32 })
        );
        let odd = &SALT[..63];
        assert_eq!(
            Verity::from_manifest(ROOT, Some(odd), 1, BLOCK),
            Err(VerityError::SaltLength { digits: 63 })
        );
        let long = format!("{SALT}00");
        assert_eq!(
            Verity::from_manifest(ROOT, Some(&long), 1, BLOCK),
            Err(VerityError::SaltLength { digits: 66 })
        );
        let upper = SALT.to_uppercase();
        assert_eq!(
            Verity::from_manifest(ROOT, Some(&upper), 1, BLOCK),
            Err(VerityError::SaltNotHex(upper))
        );
        let bad = format!("zz{}", &SALT[2..]);
        assert_eq!(
            Verity::from_manifest(ROOT, Some(&bad), 1, BLOCK),
            Err(VerityError::SaltNotHex(bad))
        );
    }

    #[test]
    fn a_root_that_is_not_a_digest_is_refused() {
        assert!(matches!(
            Verity::from_manifest("abc", Some(SALT), 1, BLOCK),
            Err(VerityError::RootHash(_))
        ));
    }

    #[test]
    fn a_block_count_of_zero_or_one_that_overflows_is_refused() {
        assert_eq!(
            Verity::from_manifest(ROOT, Some(SALT), 0, 0),
            Err(VerityError::DataBlocks(0))
        );
        assert_eq!(
            Verity::from_manifest(ROOT, Some(SALT), u64::MAX, 0),
            Err(VerityError::DataBlocks(u64::MAX))
        );
    }

    #[test]
    fn an_offset_that_is_not_the_end_of_the_data_is_refused() {
        assert_eq!(
            Verity::from_manifest(ROOT, Some(SALT), 4, 3 * BLOCK),
            Err(VerityError::Offset {
                data_blocks: 4,
                hash_offset: 3 * BLOCK,
                expected: 4 * BLOCK,
            })
        );
    }

    #[test]
    fn a_file_without_its_whole_tree_is_refused_with_the_expected_length() {
        let v = Verity::from_manifest(ROOT, Some(SALT), 4, 4 * BLOCK).expect("fits");
        assert_eq!(
            v.check_size(4 * BLOCK),
            Err(VerityError::Size {
                data_blocks: 4,
                size: 4 * BLOCK,
                expected: 5 * BLOCK,
            })
        );
        assert!(v.check_size(6 * BLOCK).is_err());
        let fewer = Verity::from_manifest(ROOT, Some(SALT), 3, 3 * BLOCK).expect("fits");
        assert!(fewer.check_size(5 * BLOCK).is_err());
    }
}
