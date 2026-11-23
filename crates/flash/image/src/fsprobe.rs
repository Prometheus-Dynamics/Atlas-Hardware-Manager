//! What filesystem a partition holds and what it is labelled, from its first bytes.
//!
//! A partition is told apart by what the image itself says: the label of the filesystem in it.
//! The data partition's label is the one the product's slot layout states (`lemnos-data` by
//! default: the same fact Lemnos's slot-init finds it by). Only the two filesystems an image is
//! made of are known: ext2/3/4 and FAT.

/// How many of a partition's first bytes [`probe`] needs.
pub(crate) const PROBE_BYTES: usize = 2048;

/// What a partition's first bytes say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// `ext`, `vfat`, or `unknown`.
    pub(crate) filesystem: &'static str,
    /// The filesystem's label, when it has one.
    pub(crate) label: Option<String>,
}

fn label(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]).trim().to_owned();
    // mkfs.fat writes "NO NAME" for a volume with no label.
    (!text.is_empty() && text != "NO NAME").then_some(text)
}

/// Reads the filesystem and label from `head`, the partition's first bytes (up to
/// [`PROBE_BYTES`]; a shorter partition gives what it has).
pub(crate) fn probe(head: &[u8]) -> Found {
    // ext2/3/4: the superblock is at byte 1024; its magic is at +56, its label (16 bytes) at +120.
    if head.get(1024 + 56..1024 + 58) == Some(&[0x53, 0xEF]) {
        return Found {
            filesystem: "ext",
            label: head.get(1024 + 120..1024 + 136).and_then(label),
        };
    }
    // FAT: the boot sector's signature, and the type text at 54 (FAT12/16) or 82 (FAT32), with
    // the volume label 11 bytes before it.
    if head.get(510..512) == Some(&[0x55, 0xAA]) {
        if head.get(82..87) == Some(b"FAT32".as_slice()) {
            return Found {
                filesystem: "vfat",
                label: head.get(71..82).and_then(label),
            };
        }
        if head.get(54..57) == Some(b"FAT".as_slice()) {
            return Found {
                filesystem: "vfat",
                label: head.get(43..54).and_then(label),
            };
        }
    }
    Found {
        filesystem: "unknown",
        label: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ext_superblock_gives_its_label_and_one_without_a_label_gives_none() {
        let mut head = vec![0u8; PROBE_BYTES];
        head[1024 + 56] = 0x53;
        head[1024 + 57] = 0xEF;
        assert_eq!(
            probe(&head),
            Found {
                filesystem: "ext",
                label: None
            }
        );
        head[1024 + 120..1024 + 131].copy_from_slice(b"lemnos-data");
        assert_eq!(
            probe(&head),
            Found {
                filesystem: "ext",
                label: Some("lemnos-data".into())
            }
        );
    }

    #[test]
    fn a_fat_boot_sector_gives_its_label_and_a_blank_one_gives_none() {
        let mut head = vec![0u8; PROBE_BYTES];
        head[510] = 0x55;
        head[511] = 0xAA;
        head[54..62].copy_from_slice(b"FAT16   ");
        // A real boot partition as a board's build writes it today: eleven NUL bytes, no label.
        assert_eq!(
            probe(&head),
            Found {
                filesystem: "vfat",
                label: None
            }
        );
        head[43..54].copy_from_slice(b"NO NAME    ");
        assert_eq!(probe(&head).label, None);
        head[43..54].copy_from_slice(b"BOOT       ");
        assert_eq!(probe(&head).label, Some("BOOT".into()));
        let mut fat32 = vec![0u8; PROBE_BYTES];
        fat32[510] = 0x55;
        fat32[511] = 0xAA;
        fat32[82..90].copy_from_slice(b"FAT32   ");
        fat32[71..82].copy_from_slice(b"BOOT       ");
        assert_eq!(probe(&fat32).label, Some("BOOT".into()));
    }

    #[test]
    fn anything_else_is_unknown() {
        assert_eq!(probe(&[0u8; 100]).filesystem, "unknown");
        assert_eq!(probe(&vec![0xFFu8; PROBE_BYTES]).filesystem, "unknown");
    }
}
