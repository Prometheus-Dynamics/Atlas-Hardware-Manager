//! The boot partition, read and never written: its whole file tree (read through `fatfs`, names as
//! the volume holds them, looked up ignoring case, FAT being case-blind), and a bay set's committed
//! record (`bayinfo.rec`), read with Lemnos's own readers (`twocopy::decode`, `bays::Info::decode`)
//! so the record's form can't drift from the writer's.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use fatfs::{Dir, FileSystem, FsOptions, ReadWriteSeek};
use lemnos_slot_store::bays::{Info, ROLES};
use lemnos_slot_store::twocopy::{self, Kind, SECTOR, Sector};

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

/// The five payload roles of a bay set, in Lemnos's order: `lemnos_slot_store::bays::ROLES`, the one list.
pub const BAY_PATHS: [&str; 5] = [
    ROLES[0].path,
    ROLES[1].path,
    ROLES[2].path,
    ROLES[3].path,
    ROLES[4].path,
];

/// A bay set's committed record, as Lemnos decodes it: the slot, each role's stored length and
/// SHA-256, and each firmware overlay the set holds (`bays::Info`).
pub type BayInfo = Info;

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

/// Reads a bay record file (two sectors, a copy in each) as Lemnos does: each sector through
/// `twocopy::decode`, the valid `BAY_INFO` copy with the higher sequence, its payload through
/// `bays::Info::decode` (the five roles, then any firmware overlays). `None` when neither copy is a
/// valid bay record.
#[must_use]
pub fn parse_bayinfo(bytes: &[u8]) -> Option<BayInfo> {
    let mut best: Option<(u64, &[u8])> = None;
    for chunk in bytes.chunks(SECTOR).take(2) {
        let Ok(sector) = <&Sector>::try_from(chunk) else {
            break;
        };
        let Some(copy) = twocopy::decode(sector) else {
            continue;
        };
        if copy.kind != Kind::BAY_INFO {
            continue;
        }
        if best.is_none_or(|(seen, _)| copy.seq > seen) {
            best = Some((copy.seq, copy.payload));
        }
    }
    Info::decode(best?.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(slot: &str, firmware: Vec<(String, u64, [u8; 32])>) -> Info {
        Info {
            slot: slot.into(),
            files: [
                (1, [1; 32]),
                (2, [2; 32]),
                (3, [3; 32]),
                (4, [4; 32]),
                (5, [5; 32]),
            ],
            firmware,
        }
    }

    fn file(copies: &[(u64, &Info)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (seq, info) in copies {
            let mut sector: Sector = [0; SECTOR];
            twocopy::encode(Kind::BAY_INFO, *seq, &info.encode(), &mut sector).expect("encoded");
            out.extend_from_slice(&sector);
        }
        out
    }

    #[test]
    fn the_higher_valid_copy_wins_and_firmware_overlays_are_read() {
        let old = record("slot-old", vec![]);
        let new = record(
            "slot-new",
            vec![("overlays/ramoops-pi5.dtbo".into(), 745, [9; 32])],
        );
        assert_eq!(
            parse_bayinfo(&file(&[(1, &old), (2, &new)])),
            Some(new.clone())
        );
        let mut torn = file(&[(3, &old), (2, &new)]);
        torn[10] ^= 1;
        assert_eq!(parse_bayinfo(&torn), Some(new));
        assert_eq!(parse_bayinfo(&[0; 1024]), None);
    }
}
