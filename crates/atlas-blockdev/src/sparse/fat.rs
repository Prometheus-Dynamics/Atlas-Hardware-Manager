//! FAT16 and FAT32 free clusters, from the FAT. FAT has no checksums, so
//! the copies must agree (two at least) and the first entry must carry the
//! boot sector's media byte; otherwise every cluster is written. FAT12
//! (tiny volumes) is always written.

use std::ops::Range;

use super::{le16, le32};

pub(crate) struct Fat {
    start: u64,
    /// The first byte of cluster 2, in the image.
    data: u64,
    cluster: u64,
    clusters: u64,
    tables: u64,
    table_bytes: u64,
    copies: u64,
    fat32: bool,
    media: u8,
    /// Per cluster from 2: free. Empty until the FAT is read.
    free: Vec<bool>,
}

impl Fat {
    /// The FAT whose boot sector is `boot`, in the partition at `start`.
    pub(crate) fn probe(start: u64, len: u64, boot: &[u8]) -> Option<Self> {
        if boot[510..512] != [0x55, 0xAA] || !matches!(boot[0], 0xEB | 0xE9) {
            return None;
        }
        let sector = le16(boot, 11);
        let per_cluster = u64::from(boot[13]);
        let reserved = le16(boot, 14);
        let copies = u64::from(boot[16]);
        let root_entries = le16(boot, 17);
        let total = match le16(boot, 19) {
            0 => le32(boot, 32),
            small => small,
        };
        let per_fat = match le16(boot, 22) {
            0 => le32(boot, 36),
            small => small,
        };
        if !matches!(sector, 512 | 1024 | 2048 | 4096)
            || !per_cluster.is_power_of_two()
            || reserved == 0
            || copies < 2
            || per_fat == 0
            || total * sector > len
        {
            return None;
        }
        let root = (root_entries * 32).div_ceil(sector);
        let first_data = reserved + copies * per_fat + root;
        let clusters = total.checked_sub(first_data)? / per_cluster;
        let fat32 = match clusters {
            0..4085 => return None,
            4085..65525 => false,
            _ => true,
        };
        let entry = if fat32 { 4 } else { 2 };
        if (clusters + 2) * entry > per_fat * sector {
            return None;
        }
        Some(Self {
            start,
            data: start + first_data * sector,
            cluster: per_cluster * sector,
            clusters,
            tables: start + reserved * sector,
            table_bytes: per_fat * sector,
            copies,
            fat32,
            media: boot[21],
            free: Vec::new(),
        })
    }

    /// Where all copies of the FAT are.
    pub(crate) fn tables(&self) -> (u64, usize) {
        (self.tables, (self.table_bytes * self.copies) as usize)
    }

    /// Reads the FAT; false when its copies disagree or it doesn't match
    /// the boot sector.
    pub(crate) fn load_tables(&mut self, tables: &[u8]) -> bool {
        let size = self.table_bytes as usize;
        let first = &tables[..size];
        if tables.chunks_exact(size).any(|copy| copy != first) || first[0] != self.media {
            return false;
        }
        self.free = (2..self.clusters + 2)
            .map(|cluster| {
                if self.fat32 {
                    le32(first, cluster as usize * 4) & 0x0FFF_FFFF == 0
                } else {
                    le16(first, cluster as usize * 2) == 0
                }
            })
            .collect();
        true
    }

    /// Whether every cluster `range` (bytes of the image) touches is free.
    pub(crate) fn free(&self, range: Range<u64>) -> bool {
        let end = self.data + self.clusters * self.cluster;
        if self.free.is_empty() || range.start < self.data || range.end > end || range.is_empty() {
            return false;
        }
        debug_assert!(self.start <= self.data);
        let first = (range.start - self.data) / self.cluster;
        let last = (range.end - 1 - self.data) / self.cluster;
        (first..=last).all(|index| self.free[index as usize])
    }
}
