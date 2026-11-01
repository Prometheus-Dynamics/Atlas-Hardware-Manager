//! Raw flash partitions as Linux shows them (`/dev/mtdN`, named in
//! `/sys/class/mtd`), behind a trait so the slot logic can be tested on a RAM
//! flash. Writes skip bad eraseblocks as `nandwrite` does.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// One flash partition, addressed by byte offset.
pub trait Mtd {
    fn size(&self) -> u64;
    fn erase_size(&self) -> u32;
    fn page_size(&self) -> u32;
    fn is_bad(&mut self, offset: u64) -> Result<bool, String>;
    fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), String>;
    /// Erases the eraseblock at `offset`.
    fn erase(&mut self, offset: u64) -> Result<(), String>;
    /// Programs whole pages at `offset`, in an erased block.
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), String>;
}

/// The node of the partition named `name` under `sysfs` (`/sys/class/mtd`):
/// `mtd4`, not its read-only `mtd4ro`.
pub fn find_by_name(sysfs: &Path, name: &str) -> Option<String> {
    let mut nodes: Vec<String> = std::fs::read_dir(sysfs)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|node| {
            node.strip_prefix("mtd")
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .filter(|node| {
            std::fs::read_to_string(sysfs.join(node).join("name"))
                .is_ok_and(|text| text.trim() == name)
        })
        .collect();
    nodes.sort();
    nodes.into_iter().next()
}

/// Erases `mtd`'s good blocks and writes `bytes` of `image` from its start,
/// skipping bad blocks (a block's last page padded with 0xff), then reads it
/// back the same way. Refuses an image the good blocks can't hold before
/// erasing anything. Returns its SHA-256.
pub fn write_skipping_bad(
    mtd: &mut dyn Mtd,
    image: &mut dyn Read,
    bytes: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<String, String> {
    let block = u64::from(mtd.erase_size());
    let page = mtd.page_size() as usize;
    if block == 0 || page == 0 {
        return Err("the flash reports no block or page size".into());
    }
    let mut good = Vec::new();
    let mut at = 0;
    while at < mtd.size() {
        if !mtd.is_bad(at)? {
            good.push(at);
        }
        at += block;
    }
    let needed = bytes.div_ceil(block);
    if needed > good.len() as u64 {
        return Err(format!(
            "the image is {bytes} bytes; the partition has {} good {block}-byte blocks",
            good.len()
        ));
    }
    for at in &good {
        mtd.erase(*at)?;
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; block as usize];
    let mut left = bytes;
    for at in good.iter().take(needed as usize) {
        let take = left.min(block) as usize;
        image
            .read_exact(&mut buffer[..take])
            .map_err(|error| format!("reading the image: {error}"))?;
        hasher.update(&buffer[..take]);
        let padded = take.div_ceil(page) * page;
        buffer[take..padded].fill(0xff);
        mtd.write(*at, &buffer[..padded])?;
        left -= take as u64;
        progress(bytes - left);
    }
    let written = crate::volume::hex(&hasher.finalize());
    let mut hasher = Sha256::new();
    let mut left = bytes;
    for at in good.iter().take(needed as usize) {
        let take = left.min(block) as usize;
        mtd.read(*at, &mut buffer[..take])?;
        hasher.update(&buffer[..take]);
        left -= take as u64;
    }
    let read = crate::volume::hex(&hasher.finalize());
    if read != written {
        return Err(format!(
            "the partition reads back {read}, not the {written} written"
        ));
    }
    Ok(written)
}

/// A RAM flash, for tests here and in `nand_slots`.
#[cfg(test)]
pub(crate) mod ram {
    use super::Mtd;

    pub struct RamMtd {
        pub bytes: Vec<u8>,
        pub block: u32,
        pub page: u32,
        pub bad: Vec<u64>,
        /// Every erase and write, in order, as ("erase"|"write", offset).
        pub log: Vec<(&'static str, u64)>,
        /// Writes left before the flash loses power: the next one lands
        /// half written, and it and everything after it fail.
        pub power_cut_after: Option<usize>,
    }

    impl RamMtd {
        pub fn new(blocks: u32, block: u32, page: u32) -> Self {
            Self {
                bytes: vec![0xff; (blocks * block) as usize],
                block,
                page,
                bad: Vec::new(),
                log: Vec::new(),
                power_cut_after: None,
            }
        }
    }

    impl Mtd for RamMtd {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn erase_size(&self) -> u32 {
            self.block
        }
        fn page_size(&self) -> u32 {
            self.page
        }
        fn is_bad(&mut self, offset: u64) -> Result<bool, String> {
            Ok(self.bad.contains(&offset))
        }
        fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
            let at = offset as usize;
            buffer.copy_from_slice(&self.bytes[at..at + buffer.len()]);
            Ok(())
        }
        fn erase(&mut self, offset: u64) -> Result<(), String> {
            assert_eq!(offset % u64::from(self.block), 0);
            if self.power_cut_after == Some(0) {
                return Err("power cut".into());
            }
            self.log.push(("erase", offset));
            let at = offset as usize;
            self.bytes[at..at + self.block as usize].fill(0xff);
            Ok(())
        }
        fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), String> {
            assert_eq!(data.len() % self.page as usize, 0, "whole pages");
            let at = offset as usize;
            if let Some(left) = self.power_cut_after {
                if left == 0 {
                    // A torn write: the first half landed.
                    let half = data.len() / 2;
                    self.bytes[at..at + half].copy_from_slice(&data[..half]);
                    return Err("power cut".into());
                }
                self.power_cut_after = Some(left - 1);
            }
            self.log.push(("write", offset));
            self.bytes[at..at + data.len()].copy_from_slice(data);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ram::RamMtd;
    use super::*;

    #[test]
    fn bad_blocks_are_skipped_and_a_full_partition_refused() {
        let mut mtd = RamMtd::new(4, 1024, 256);
        mtd.bad.push(1024);
        let image: Vec<u8> = (0..1500u32).map(|i| (i % 251) as u8).collect();
        let sha = write_skipping_bad(&mut mtd, &mut &image[..], 1500, &mut |_| {}).unwrap();
        assert_eq!(
            sha,
            crate::volume::hash_prefix(&mut &image[..], 1500).unwrap()
        );
        assert_eq!(&mtd.bytes[..1024], &image[..1024]);
        assert!(
            mtd.bytes[1024..2048].iter().all(|b| *b == 0xff),
            "the bad block"
        );
        assert_eq!(&mtd.bytes[2048..2048 + 476], &image[1024..]);
        // The last block's partial page is padded with erased bytes.
        assert_eq!(mtd.bytes[2048 + 476], 0xff);

        let mut small = RamMtd::new(2, 1024, 256);
        small.bad.push(0);
        let error = write_skipping_bad(&mut small, &mut &image[..], 1500, &mut |_| {}).unwrap_err();
        assert!(error.contains("good"), "{error}");
        assert!(small.log.is_empty(), "nothing erased");
    }

    #[test]
    fn partitions_are_found_by_name_not_read_only_nodes() {
        let root = std::env::temp_dir().join(format!("atlas-ubi-mtd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (node, name) in [("mtd0", "env"), ("mtd0ro", "env"), ("mtd4", "boot_b")] {
            std::fs::create_dir_all(root.join(node)).unwrap();
            std::fs::write(root.join(node).join("name"), format!("{name}\n")).unwrap();
        }
        assert_eq!(find_by_name(&root, "env").as_deref(), Some("mtd0"));
        assert_eq!(find_by_name(&root, "boot_b").as_deref(), Some("mtd4"));
        assert_eq!(find_by_name(&root, "boot_a"), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
