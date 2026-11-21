//! The boot partition, read and never written: its whole file tree (read through `fatfs`, names as
//! the volume holds them, looked up ignoring case, FAT being case-blind), and a bay set's committed
//! record (`bayinfo.rec`), read with Lemnos's own readers (`twocopy::decode`, `bays::Info::decode`)
//! so the record's form can't drift from the writer's.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use fatfs::{Dir, FileSystem, FsOptions, ReadWriteSeek};
use lemnos_slot_store::bays::{FIRMWARE_LIST, Info, Layout, parse_firmware_list};
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

/// A bay set's committed record, as Lemnos decodes it (layout v6): the slot, each written file's
/// stored length and SHA-256 prefix, and the list digest (`bays::Info`).
pub type BayInfo = Info;

/// The bay layout the boot partition's root `firmware-overlays.txt` declares, read with Lemnos's
/// own parser: the device tree names, firmware overlays and files, the config digest and the form.
/// Only the names are used; the sizes are each record's own.
///
/// # Errors
///
/// A partition without the file, or a file Lemnos's parser refuses.
pub fn layout(tree: &Tree) -> Result<Layout, String> {
    let (_, bytes) = find(tree, FIRMWARE_LIST).ok_or_else(|| {
        format!("the boot partition has no {FIRMWARE_LIST} (a layout v6 image names its bay members there)")
    })?;
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{FIRMWARE_LIST} is not text"))?;
    let declared = parse_firmware_list(text)?;
    let sector = SECTOR as u64;
    Ok(Layout::new([sector; 5])?
        .with_dtbs(&declared.dtbs)
        .with_firmware(&declared.overlays, declared.config)
        .with_files(&declared.files)
        .with_form(declared.form))
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

/// Reads a bay record file (two sectors, a copy in each) as Lemnos does: each sector through
/// `twocopy::decode`, the valid `BAY_INFO` copy with the higher sequence, its payload through
/// `bays::Info::decode` (layout v6). `None` when neither copy is a
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
    use lemnos_slot_store::bayfat::Region;

    fn record(slot: &str, files: &[(&str, &[u8])]) -> Info {
        let files: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(p, b)| ((*p).to_string(), b.to_vec()))
            .collect();
        Info::committed(
            Region {
                start: 3,
                clusters: 40,
            },
            slot,
            &files,
        )
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
        let old = record("slot-old", &[("kernel8.img", b"k")]);
        let new = record(
            "slot-new",
            &[("kernel8.img", b"k2"), ("overlays/ramoops-pi5.dtbo", b"o")],
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
