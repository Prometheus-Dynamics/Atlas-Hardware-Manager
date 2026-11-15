//! The boot partition, read and never written: its whole file tree (read through `fatfs`, names
//! lower-cased, FAT being case-blind), and a bay set's committed record (`bayinfo.rec`, the
//! two-copy format of `lemnos_slot_store::twocopy`, read the same way: the valid copy with the
//! higher sequence).

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use fatfs::{Dir, FileSystem, FsOptions, ReadWriteSeek};
use lemnos_slot_store::bays::ROLES;

/// The boot partition's files: each path (`/` between directories) with the name as the volume
/// holds it, to the file's bytes. Names are never rewritten; lookups are case-insensitive.
pub type Tree = BTreeMap<String, Vec<u8>>;

/// The entry whose path matches `path` ignoring ASCII case: the path as the volume holds it.
#[must_use]
pub fn find<'a>(tree: &'a Tree, path: &str) -> Option<(&'a str, &'a [u8])> {
    tree.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(path))
        .map(|(key, bytes)| (key.as_str(), bytes.as_slice()))
}

/// Paths of the tree that collide with another path ignoring ASCII case: each group named.
#[must_use]
pub fn collisions(tree: &Tree) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in tree.keys() {
        groups
            .entry(key.to_ascii_lowercase())
            .or_default()
            .push(key.clone());
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

/// The bay record's kind byte (Lemnos `Kind::BAY_INFO`).
const BAY_INFO_KIND: u8 = 6;
const SECTOR: usize = 512;
const CRC_AT: usize = 508;
const HEADER: usize = 18;
/// The five payload roles of a bay set, in Lemnos's order: `lemnos_slot_store::bays::ROLES`, the one list.
pub const BAY_PATHS: [&str; 5] = [
    ROLES[0].path,
    ROLES[1].path,
    ROLES[2].path,
    ROLES[3].path,
    ROLES[4].path,
];

/// A bay set's committed record: the slot in it and each payload's stored length and SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BayInfo {
    pub slot: String,
    /// In [`BAY_PATHS`] order: (stored length, SHA-256 of the stored bytes).
    pub files: Vec<(u64, [u8; 32])>,
}

/// Reads the whole file tree of a FAT boot partition image, names as the volume holds them.
///
/// # Errors
///
/// An image that `fatfs` does not read as a FAT volume, or a directory it cannot list.
pub fn read_tree(image: Vec<u8>) -> Result<Tree, String> {
    let fs = FileSystem::new(Cursor::new(image), FsOptions::new())
        .map_err(|error| format!("not a FAT boot partition: {error}"))?;
    let mut tree = Tree::new();
    walk(fs.root_dir(), "", &mut tree)?;
    Ok(tree)
}

fn walk<IO: ReadWriteSeek>(dir: Dir<'_, IO>, prefix: &str, tree: &mut Tree) -> Result<(), String> {
    for entry in dir.iter() {
        let entry = entry.map_err(|error| format!("listing {prefix:?}: {error}"))?;
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let path = format!("{prefix}{name}");
        if entry.is_dir() {
            walk(entry.to_dir(), &format!("{path}/"), tree)?;
        } else {
            let mut bytes = Vec::new();
            entry
                .to_file()
                .read_to_end(&mut bytes)
                .map_err(|error| format!("reading {path}: {error}"))?;
            tree.insert(path, bytes);
        }
    }
    Ok(())
}

/// The CRC-32 (IEEE, reflected) of `bytes`, as the two-copy records use.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Reads a bay record file (1024 bytes, two sectors): the valid copy with the higher sequence,
/// decoded. `None` when neither copy is a valid bay record.
#[must_use]
pub fn parse_bayinfo(bytes: &[u8]) -> Option<BayInfo> {
    let mut best: Option<(u64, &[u8])> = None;
    for at in [0, SECTOR] {
        let Some(copy) = bytes.get(at..at + SECTOR) else {
            break;
        };
        if &copy[0..4] != b"LMRC" || copy[4] != 1 || copy[5] != BAY_INFO_KIND {
            continue;
        }
        let crc = u32::from_le_bytes(copy[CRC_AT..CRC_AT + 4].try_into().ok()?);
        if crc != crc32(&copy[..CRC_AT]) {
            continue;
        }
        let sequence = u64::from_le_bytes(copy[8..16].try_into().ok()?);
        let len = usize::from(u16::from_le_bytes(copy[16..18].try_into().ok()?));
        let payload = copy.get(HEADER..HEADER + len)?;
        if best.is_none_or(|(seen, _)| sequence > seen) {
            best = Some((sequence, payload));
        }
    }
    let (_, payload) = best?;
    decode_payload(payload)
}

fn decode_payload(payload: &[u8]) -> Option<BayInfo> {
    let (&n, rest) = payload.split_first()?;
    let n = usize::from(n);
    let slot = std::str::from_utf8(rest.get(..n)?).ok()?.to_string();
    let mut at = n;
    let mut files = Vec::with_capacity(BAY_PATHS.len());
    for _ in BAY_PATHS {
        let len = u64::from_le_bytes(rest.get(at..at + 8)?.try_into().ok()?);
        let hash: [u8; 32] = rest.get(at + 8..at + 40)?.try_into().ok()?;
        files.push((len, hash));
        at += 40;
    }
    (at == rest.len()).then_some(BayInfo { slot, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_ieee_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
