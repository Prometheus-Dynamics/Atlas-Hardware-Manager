//! The GUID partition table at the start of a first-flash image: the header at
//! LBA 1 and its entry array, both checked with CRC-32. Only the primary table
//! is read; the backup at the end of the disk is not needed to read a slot.

use crate::error::ImageError;

/// The logical sector size. Gaia's disks are 512-byte sectors.
pub(crate) const SECTOR: u64 = 512;
/// The most entries a GPT holds here: the standard array.
const MAX_ENTRIES: u32 = 128;
/// The largest entry array read (128 entries of 512 bytes).
const MAX_ENTRY_TABLE: u64 = 64 * 1024;
/// Where the partition table must end. The head of the image is held in memory
/// until it does, so this is the cap on that buffer.
pub(crate) const MAX_HEAD: u64 = 1 << 20;

const SIGNATURE: &[u8] = b"EFI PART";

/// The checked header: where the entry array is and how big.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    entries_start: u64,
    entries: u32,
    entry_size: u32,
    entries_crc: u32,
}

impl Header {
    /// The byte just past the entry array: partitions start at or after it.
    pub(crate) fn end(&self) -> u64 {
        self.entries_start + u64::from(self.entries) * u64::from(self.entry_size)
    }
}

/// One partition as the table gives it: its name and where its bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) start: u64,
    pub(crate) len: u64,
}

fn table(why: impl Into<String>) -> ImageError {
    ImageError::Table(why.into())
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

fn le_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let raw: [u8; 8] = bytes.get(at..at + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(raw))
}

/// Reads and checks the header. `head` is the start of the image and must hold
/// at least 1024 bytes (the first two sectors).
pub(crate) fn header(head: &[u8]) -> Result<Header, ImageError> {
    let h = head
        .get(512..1024)
        .ok_or_else(|| table("the image is shorter than two sectors"))?;
    if h.get(0..8) != Some(SIGNATURE) {
        return Err(table("there is no GPT header at LBA 1"));
    }
    let size = le_u32(h, 12).ok_or_else(|| table("the header is truncated"))? as usize;
    if !(92..=512).contains(&size) {
        return Err(table(format!("the header size {size} is not 92 to 512")));
    }
    let mut checked = head
        .get(512..512 + size)
        .ok_or_else(|| table("the header is truncated"))?
        .to_vec();
    checked[16..20].fill(0);
    let stored = le_u32(h, 16).ok_or_else(|| table("the header is truncated"))?;
    if crc32(&checked) != stored {
        return Err(table("the header's CRC does not match"));
    }
    if le_u64(h, 24) != Some(1) {
        return Err(table("the header is not the primary one at LBA 1"));
    }
    let entries_lba = le_u64(h, 72).ok_or_else(|| table("the header is truncated"))?;
    let entries = le_u32(h, 80).ok_or_else(|| table("the header is truncated"))?;
    let entry_size = le_u32(h, 84).ok_or_else(|| table("the header is truncated"))?;
    let entries_crc = le_u32(h, 88).ok_or_else(|| table("the header is truncated"))?;
    if !(1..=MAX_ENTRIES).contains(&entries) {
        return Err(table(format!(
            "{entries} partition entries is not 1 to 128"
        )));
    }
    if !matches!(entry_size, 128 | 256 | 512) {
        return Err(table(format!(
            "the entry size {entry_size} is not 128, 256 or 512"
        )));
    }
    let entries_start = entries_lba
        .checked_mul(SECTOR)
        .ok_or_else(|| table("the entry array's place overflows"))?;
    if entries_start < 1024 {
        return Err(table("the entry array overlaps the header"));
    }
    let length = u64::from(entries) * u64::from(entry_size);
    if length > MAX_ENTRY_TABLE {
        return Err(table("the entry array is larger than 64 KiB"));
    }
    let header = Header {
        entries_start,
        entries,
        entry_size,
        entries_crc,
    };
    if header.end() > MAX_HEAD {
        return Err(table("the partition table ends past 1 MiB"));
    }
    Ok(header)
}

/// Reads the partitions from the entry array. `head` must reach `header.end()`.
pub(crate) fn entries(head: &[u8], header: &Header) -> Result<Vec<Entry>, ImageError> {
    let start = usize::try_from(header.entries_start).map_err(|_| table("too far"))?;
    let end = usize::try_from(header.end()).map_err(|_| table("too far"))?;
    let array = head
        .get(start..end)
        .ok_or_else(|| table("the entry array is truncated"))?;
    if crc32(array) != header.entries_crc {
        return Err(table("the entry array's CRC does not match"));
    }
    let size = header.entry_size as usize;
    let mut out = Vec::new();
    for index in 0..header.entries as usize {
        let entry = array
            .get(index * size..(index + 1) * size)
            .ok_or_else(|| table("the entry array is truncated"))?;
        if entry
            .get(0..16)
            .is_none_or(|guid| guid.iter().all(|b| *b == 0))
        {
            continue;
        }
        let first = le_u64(entry, 32).ok_or_else(|| table("an entry is truncated"))?;
        let last = le_u64(entry, 40).ok_or_else(|| table("an entry is truncated"))?;
        if first > last {
            return Err(table(format!("entry {index} ends before it starts")));
        }
        let start = first
            .checked_mul(SECTOR)
            .ok_or_else(|| table(format!("entry {index} starts past the end of a disk")))?;
        if start < header.end() {
            return Err(table(format!(
                "entry {index} starts inside the partition table"
            )));
        }
        let len = (last - first + 1)
            .checked_mul(SECTOR)
            .ok_or_else(|| table(format!("entry {index} is too long")))?;
        let name = utf16_name(entry.get(56..128).unwrap_or_default())
            .ok_or_else(|| table(format!("entry {index}'s name is not UTF-16")))?;
        out.push(Entry { name, start, len });
    }
    out.sort_by_key(|entry| entry.start);
    for pair in out.windows(2) {
        if pair[0].start + pair[0].len > pair[1].start {
            return Err(table(format!(
                "partitions {:?} and {:?} overlap",
                pair[0].name, pair[1].name
            )));
        }
    }
    Ok(out)
}

fn utf16_name(bytes: &[u8]) -> Option<String> {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16(&units).ok()
}

/// CRC-32 as GPT uses it (IEEE 802.3, reflected).
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_name_stops_at_its_terminator_and_refuses_bad_utf16() {
        let mut bytes = Vec::new();
        for unit in "data".encode_utf16().chain([0, 0x41]) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(utf16_name(&bytes).as_deref(), Some("data"));
        assert_eq!(utf16_name(&[0x00, 0xD8]), None);
    }
}
