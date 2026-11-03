//! ext4's free blocks, from its block bitmaps. Only with metadata
//! checksums: the superblock, every group descriptor and every bitmap must
//! check out, and every feature must be one whose bitmaps mean what they
//! say. A group whose bitmap isn't initialised (BLOCK_UNINIT) or doesn't
//! check out keeps all its blocks written.

use std::ops::Range;

use super::{crc32c, le16, le32};

const MAGIC: u64 = 0xEF53;
/// Superblock flag: the bitmaps carry crc32c checksums.
const RO_METADATA_CSUM: u64 = 0x400;
/// Read-only features whose bitmaps are plain block bitmaps: sparse_super,
/// large_file, btree_dir, huge_file, gdt_csum, dir_nlink, extra_isize,
/// quota, metadata_csum, readonly, project, verity, orphan_present. Not
/// has_snapshot, bigalloc (cluster bitmaps), replica or shared_blocks.
const RO_KNOWN: u64 =
    0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x100 | 0x400 | 0x1000 | 0x2000 | 0x8000 | 0x10000;
/// Incompatible features that leave the group layout alone: filetype,
/// extent, 64bit, mmp, flex_bg, ea_inode, csum_seed, largedir,
/// inline_data, encrypt, casefold. Not recover (a journal to replay),
/// journal_dev, meta_bg (descriptors elsewhere) or dirdata.
const INCOMPAT_KNOWN: u64 =
    0x2 | 0x40 | 0x80 | 0x100 | 0x200 | 0x400 | 0x2000 | 0x4000 | 0x8000 | 0x10000 | 0x20000;
const INCOMPAT_64BIT: u64 = 0x80;
const INCOMPAT_CSUM_SEED: u64 = 0x2000;
const BLOCK_UNINIT: u64 = 0x2;
/// `s_state`: errors were found.
const STATE_ERRORS: u64 = 0x2;

pub(crate) struct Ext4 {
    start: u64,
    block_size: u64,
    blocks: u64,
    first_data_block: u64,
    blocks_per_group: u64,
    desc_size: usize,
    is_64bit: bool,
    seed: u32,
    /// Per group: its block bitmap, once read and checked.
    bitmaps: Vec<Option<Vec<u8>>>,
    /// Per group: where its bitmap is and its checksum, from its descriptor.
    pending: Vec<Option<(u64, u32)>>,
}

impl Ext4 {
    /// The filesystem whose superblock is `sb`, in the partition at `start`.
    pub(crate) fn probe(start: u64, len: u64, sb: &[u8]) -> Option<Self> {
        if le16(sb, 0x38) != MAGIC || le16(sb, 0x3A) & STATE_ERRORS != 0 {
            return None;
        }
        let incompat = le32(sb, 0x60);
        let ro = le32(sb, 0x64);
        if incompat & !INCOMPAT_KNOWN != 0 || ro & !RO_KNOWN != 0 || ro & RO_METADATA_CSUM == 0 {
            return None;
        }
        // crc32c, over the superblock up to its checksum.
        if sb[0x175] != 1 || u64::from(crc32c(!0, &sb[..0x3FC])) != le32(sb, 0x3FC) {
            return None;
        }
        let log = le32(sb, 0x18);
        let blocks_per_group = le32(sb, 0x20);
        if log > 6 || blocks_per_group == 0 || le32(sb, 0x24) != blocks_per_group {
            return None;
        }
        let block_size = 1024 << log;
        if blocks_per_group != block_size * 8 {
            return None;
        }
        let is_64bit = incompat & INCOMPAT_64BIT != 0;
        let blocks = le32(sb, 0x4) | if is_64bit { le32(sb, 0x150) << 32 } else { 0 };
        let first_data_block = le32(sb, 0x14);
        let desc_size = if is_64bit {
            le16(sb, 0xFE) as usize
        } else {
            32
        };
        if blocks.checked_mul(block_size)? > len
            || first_data_block >= blocks
            || !(32..=1024).contains(&desc_size)
            || (is_64bit && desc_size < 64)
        {
            return None;
        }
        let seed = if incompat & INCOMPAT_CSUM_SEED != 0 {
            le32(sb, 0x270) as u32
        } else {
            crc32c(!0, &sb[0x68..0x78])
        };
        let groups = (blocks - first_data_block).div_ceil(blocks_per_group) as usize;
        Some(Self {
            start,
            block_size,
            blocks,
            first_data_block,
            blocks_per_group,
            desc_size,
            is_64bit,
            seed,
            bitmaps: vec![None; groups],
            pending: vec![None; groups],
        })
    }

    /// Where the group descriptors are: the block after the superblock's.
    pub(crate) fn groups(&self) -> (u64, usize) {
        (
            self.start + (self.first_data_block + 1) * self.block_size,
            self.bitmaps.len() * self.desc_size,
        )
    }

    /// Reads the descriptors; false when one doesn't check out.
    pub(crate) fn load_groups(&mut self, table: &[u8]) -> bool {
        for (group, desc) in table.chunks_exact(self.desc_size).enumerate() {
            let mut crc = crc32c(self.seed, &(group as u32).to_le_bytes());
            crc = crc32c(crc, &desc[..0x1E]);
            crc = crc32c(crc, &[0, 0]);
            crc = crc32c(crc, &desc[0x20..]);
            if u64::from(crc & 0xFFFF) != le16(desc, 0x1E) {
                return false;
            }
            if le16(desc, 0x12) & BLOCK_UNINIT != 0 {
                continue;
            }
            let block = le32(desc, 0)
                | if self.is_64bit {
                    le32(desc, 0x20) << 32
                } else {
                    0
                };
            if block >= self.blocks {
                return false;
            }
            let mut sum = le16(desc, 0x18);
            if self.desc_size >= 0x3A {
                sum |= le16(desc, 0x38) << 16;
            }
            self.pending[group] = Some((block, sum as u32));
        }
        true
    }

    /// The bitmaps to read: (group, offset, length).
    pub(crate) fn bitmap_reads(&self) -> Vec<(usize, u64, usize)> {
        self.pending
            .iter()
            .enumerate()
            .filter_map(|(group, pending)| {
                pending.map(|(block, _)| {
                    (
                        group,
                        self.start + block * self.block_size,
                        self.block_size as usize,
                    )
                })
            })
            .collect()
    }

    /// Keeps a group's bitmap when its checksum checks out.
    pub(crate) fn load_bitmap(&mut self, group: usize, bitmap: &[u8]) {
        let Some((_, sum)) = self.pending[group] else {
            return;
        };
        let bitmap = &bitmap[..(self.blocks_per_group / 8) as usize];
        let crc = crc32c(self.seed, bitmap);
        let crc = if self.desc_size >= 0x3A {
            crc
        } else {
            crc & 0xFFFF
        };
        if crc == sum {
            self.bitmaps[group] = Some(bitmap.to_vec());
        }
    }

    /// Whether every block `range` (bytes of the image) touches lies in this
    /// filesystem and is unallocated.
    pub(crate) fn free(&self, range: Range<u64>) -> bool {
        let end = self.start + self.blocks * self.block_size;
        if range.start < self.start || range.end > end || range.is_empty() {
            return false;
        }
        let first = (range.start - self.start) / self.block_size;
        let last = (range.end - 1 - self.start) / self.block_size;
        (first..=last).all(|block| {
            if block < self.first_data_block {
                return false;
            }
            let index = block - self.first_data_block;
            let group = (index / self.blocks_per_group) as usize;
            let bit = (index % self.blocks_per_group) as usize;
            self.bitmaps[group]
                .as_ref()
                .is_some_and(|bitmap| bitmap[bit / 8] & (1 << (bit % 8)) == 0)
        })
    }
}
