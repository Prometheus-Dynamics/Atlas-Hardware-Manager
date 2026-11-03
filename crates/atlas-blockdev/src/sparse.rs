//! Free space an image write may skip. Most of a board image is the empty
//! part of its filesystems: zeros that the write and the read-back would
//! otherwise spend minutes on. A zero block is not always free space,
//! though (a file's own zeros, metadata that must read as zero, a verity
//! tree), so a block is skipped only when it is all zeros AND the
//! filesystem it lies in says, through its own allocation map, that it is
//! unallocated: an ext4 block bitmap that carries a valid checksum, or a
//! FAT whose copies agree. Everything else is written in full: the
//! partition table, gaps, unknown or read-only filesystems, and any
//! partition whose maps don't parse with certainty.
//!
//! The maps are read from the image as it streams past, so nothing is
//! decoded twice. A map that only comes after a block (a bitmap late in
//! the image) leaves that block written.

use std::ops::Range;

mod ext4;
mod fat;

use ext4::Ext4;
use fat::Fat;

/// The unit skipped or written.
pub(crate) const BLOCK: usize = 4096;
/// The partition table's sector size.
const SECTOR: u64 = 512;

/// What a set of bytes from the image is wanted for.
enum Next {
    Mbr,
    /// The start of a partition: a FAT boot sector or an ext4 superblock.
    Probe {
        start: u64,
        len: u64,
    },
    Ext4Groups(usize),
    Ext4Bitmap {
        region: usize,
        group: usize,
    },
    FatTables(usize),
}

/// Bytes at `offset` still to be gathered from the stream.
struct Want {
    offset: u64,
    bytes: Vec<u8>,
    filled: usize,
    next: Next,
}

enum Region {
    /// Waiting for its tables.
    Pending,
    Ext4(Ext4),
    Fat(Fat),
}

/// The image's free space, learned as it streams past.
#[derive(Default)]
pub(crate) struct FreeMap {
    wants: Vec<Want>,
    regions: Vec<Region>,
    /// Bytes of the image already gone past.
    seen: u64,
    /// The partition table didn't parse with certainty: nothing is free.
    off: bool,
}

impl FreeMap {
    pub(crate) fn new() -> Self {
        let mut map = Self::default();
        map.want(0, SECTOR as usize, Next::Mbr);
        map
    }

    fn want(&mut self, offset: u64, len: usize, next: Next) {
        self.wants.push(Want {
            offset,
            bytes: vec![0; len],
            filled: 0,
            next,
        });
    }

    /// Takes in the image's bytes at `offset`, in order, before any of them
    /// is asked about.
    pub(crate) fn observe(&mut self, offset: u64, data: &[u8]) {
        let end = offset + data.len() as u64;
        // Parsing one want can add others in the same bytes.
        loop {
            let mut done = Vec::new();
            let mut index = 0;
            while index < self.wants.len() {
                let want = &mut self.wants[index];
                let want_end = want.offset + want.bytes.len() as u64;
                let need = want.offset + want.filled as u64;
                if need < offset {
                    // Its bytes went past before it was wanted.
                    self.wants.swap_remove(index);
                    continue;
                }
                if need < end {
                    let take = (want_end.min(end) - need) as usize;
                    let from = (need - offset) as usize;
                    want.bytes[want.filled..want.filled + take]
                        .copy_from_slice(&data[from..from + take]);
                    want.filled += take;
                }
                if want.filled == want.bytes.len() {
                    done.push(self.wants.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            if done.is_empty() {
                break;
            }
            for want in done {
                self.parse(want);
            }
        }
        self.seen = self.seen.max(end);
    }

    fn parse(&mut self, want: Want) {
        let bytes = &want.bytes;
        match want.next {
            Next::Mbr => self.partitions(bytes),
            Next::Probe { start, len } => {
                let region = self.regions.len();
                if let Some(fat) = Fat::probe(start, len, bytes) {
                    let (offset, len) = fat.tables();
                    self.regions.push(Region::Fat(fat));
                    self.want(offset, len, Next::FatTables(region));
                } else if let Some(ext4) = Ext4::probe(start, len, &bytes[1024..]) {
                    let (offset, len) = ext4.groups();
                    self.regions.push(Region::Ext4(ext4));
                    self.want(offset, len, Next::Ext4Groups(region));
                }
            }
            Next::FatTables(region) => {
                if let Region::Fat(fat) = &mut self.regions[region]
                    && !fat.load_tables(bytes)
                {
                    self.regions[region] = Region::Pending;
                }
            }
            Next::Ext4Groups(region) => {
                let mut reads = Vec::new();
                if let Region::Ext4(ext4) = &mut self.regions[region] {
                    if ext4.load_groups(bytes) {
                        reads = ext4.bitmap_reads();
                    } else {
                        self.regions[region] = Region::Pending;
                    }
                }
                for (group, offset, len) in reads {
                    self.want(offset, len, Next::Ext4Bitmap { region, group });
                }
            }
            Next::Ext4Bitmap { region, group } => {
                if let Region::Ext4(ext4) = &mut self.regions[region] {
                    ext4.load_bitmap(group, bytes);
                }
            }
        }
    }

    /// The MBR's primary partitions. Anything uncertain (no signature, a GPT,
    /// partitions that overlap or run past the disk's 2 TiB limit) turns
    /// skipping off; extended partitions are simply written.
    fn partitions(&mut self, mbr: &[u8]) {
        if mbr[510..512] != [0x55, 0xAA] {
            self.off = true;
            return;
        }
        let mut parts: Vec<Range<u64>> = Vec::new();
        for entry in mbr[446..510].as_chunks::<16>().0 {
            let kind = entry[4];
            let start = u64::from(u32::from_le_bytes(entry[8..12].try_into().unwrap()));
            let sectors = u64::from(u32::from_le_bytes(entry[12..16].try_into().unwrap()));
            match kind {
                0 => {}
                0xEE => {
                    self.off = true;
                    return;
                }
                0x05 | 0x0F | 0x85 => {}
                _ if start > 0 && sectors > 0 => {
                    parts.push(start * SECTOR..(start + sectors) * SECTOR)
                }
                _ => {}
            }
        }
        let mut sorted = parts.clone();
        sorted.sort_by_key(|part| part.start);
        if sorted.windows(2).any(|pair| pair[0].end > pair[1].start) {
            self.off = true;
            return;
        }
        for part in parts {
            // Room for a FAT boot sector and an ext4 superblock alike.
            self.want(
                part.start,
                2048,
                Next::Probe {
                    start: part.start,
                    len: part.end - part.start,
                },
            );
        }
    }

    /// Whether `range` of the image is free space its filesystem doesn't
    /// need written.
    pub(crate) fn free(&self, range: Range<u64>) -> bool {
        !self.off
            && self.regions.iter().any(|region| match region {
                Region::Pending => false,
                Region::Ext4(ext4) => ext4.free(range.clone()),
                Region::Fat(fat) => fat.free(range.clone()),
            })
    }
}

/// The image's blocks in `segment` (at `offset`) that must be written, as
/// runs of byte ranges within the segment, and the bytes skipped.
pub(crate) fn runs(map: &FreeMap, offset: u64, segment: &[u8]) -> (Vec<Range<usize>>, u64) {
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut skipped = 0;
    for (index, block) in segment.chunks(BLOCK).enumerate() {
        let start = index * BLOCK;
        let at = offset + start as u64;
        if block.iter().all(|byte| *byte == 0) && map.free(at..at + block.len() as u64) {
            skipped += block.len() as u64;
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.end == start => run.end = start + block.len(),
            _ => runs.push(start..start + block.len()),
        }
    }
    (runs, skipped)
}

/// CRC-32C as Linux computes it for ext4: no inversion in or out.
pub(crate) fn crc32c(mut crc: u32, data: &[u8]) -> u32 {
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0x82F6_3B78
            } else {
                crc >> 1
            };
        }
    }
    crc
}

pub(crate) fn le16(bytes: &[u8], at: usize) -> u64 {
    u64::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

pub(crate) fn le32(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()))
}

#[cfg(test)]
mod tests;
