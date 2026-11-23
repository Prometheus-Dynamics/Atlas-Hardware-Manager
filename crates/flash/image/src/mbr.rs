//! The MBR partition table: four primary entries in the first sector.
//!
//! An MBR names no partition. It gives each one a type byte and where its bytes are; which
//! partition is which is read from the filesystems themselves (`fsprobe`), never from a position.
//! Extended partitions are not read: an image that has one is refused.

use crate::error::ImageError;

const SECTOR: u64 = 512;
/// The type byte of a protective MBR's one entry: the disk is GPT.
const PROTECTIVE: u8 = 0xEE;
const EXTENDED: [u8; 3] = [0x05, 0x0F, 0x85];

/// One primary partition: its number (1 to 4), type byte, and where its bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) number: u8,
    pub(crate) kind: u8,
    pub(crate) start: u64,
    pub(crate) len: u64,
}

fn table(why: impl Into<String>) -> ImageError {
    ImageError::Table(why.into())
}

/// Whether the first sector carries the MBR signature.
pub(crate) fn signed(head: &[u8]) -> bool {
    head.get(510..512) == Some(&[0x55, 0xAA])
}

/// Whether the first sector is a protective MBR (the disk's table is GPT).
pub(crate) fn protective(head: &[u8]) -> bool {
    signed(head) && (0..4).any(|i| head.get(446 + 16 * i + 4) == Some(&PROTECTIVE))
}

/// The primary partitions of the MBR in `head` (its first 512 bytes), in table order.
pub(crate) fn entries(head: &[u8]) -> Result<Vec<Entry>, ImageError> {
    if !signed(head) {
        return Err(table("there is no MBR signature in the first sector"));
    }
    let mut found = Vec::new();
    for index in 0..4u8 {
        let at = 446 + 16 * usize::from(index);
        let entry = head
            .get(at..at + 16)
            .ok_or_else(|| table("the MBR is cut short"))?;
        let kind = entry[4];
        if kind == 0 {
            continue;
        }
        if EXTENDED.contains(&kind) {
            return Err(table(format!(
                "partition {} is an extended partition (type {kind:#04x}), which is not read",
                index + 1
            )));
        }
        let start = u64::from(u32::from_le_bytes([
            entry[8], entry[9], entry[10], entry[11],
        ]));
        let sectors = u64::from(u32::from_le_bytes([
            entry[12], entry[13], entry[14], entry[15],
        ]));
        if start == 0 || sectors == 0 {
            return Err(table(format!(
                "partition {} (type {kind:#04x}) has no start or no length",
                index + 1
            )));
        }
        found.push(Entry {
            number: index + 1,
            kind,
            start: start * SECTOR,
            len: sectors * SECTOR,
        });
    }
    if found.is_empty() {
        return Err(table("the MBR lists no partition"));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sector(entries: &[(u8, u32, u32)]) -> Vec<u8> {
        let mut head = vec![0u8; 512];
        head[510] = 0x55;
        head[511] = 0xAA;
        for (index, (kind, start, sectors)) in entries.iter().enumerate() {
            let at = 446 + 16 * index;
            head[at + 4] = *kind;
            head[at + 8..at + 12].copy_from_slice(&start.to_le_bytes());
            head[at + 12..at + 16].copy_from_slice(&sectors.to_le_bytes());
        }
        head
    }

    #[test]
    fn the_primary_entries_are_read_in_bytes_and_empty_ones_are_skipped() {
        let found = entries(&sector(&[(0x0C, 2048, 161_792), (0x83, 163_840, 77_824)])).unwrap();
        assert_eq!(
            found,
            [
                Entry {
                    number: 1,
                    kind: 0x0C,
                    start: 2048 * 512,
                    len: 161_792 * 512
                },
                Entry {
                    number: 2,
                    kind: 0x83,
                    start: 163_840 * 512,
                    len: 77_824 * 512
                },
            ]
        );
    }

    #[test]
    fn an_extended_partition_an_empty_table_and_a_missing_signature_are_refused() {
        let extended = entries(&sector(&[(0x0C, 2048, 100), (0x05, 4096, 100)])).unwrap_err();
        assert!(
            extended.to_string().contains("extended partition"),
            "{extended}"
        );
        let empty = entries(&sector(&[])).unwrap_err();
        assert!(empty.to_string().contains("lists no partition"), "{empty}");
        let mut unsigned = sector(&[(0x83, 2048, 100)]);
        unsigned[511] = 0;
        assert!(entries(&unsigned).is_err());
        assert!(protective(&sector(&[(0xEE, 1, 100)])));
        assert!(!protective(&sector(&[(0x83, 2048, 100)])));
    }
}
