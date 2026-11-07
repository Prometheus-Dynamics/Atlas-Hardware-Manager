//! Writing a UBI image to raw NAND the way `ubiformat -f` does, not as
//! `nandwrite` would:
//!
//! - bad eraseblocks are skipped, and one that fails to erase or program is
//!   marked bad and the PEB goes to the next good one;
//! - erase counters survive: each PEB's own counter plus one, or the mean
//!   of the flash's known counters where it had none, rewritten into the
//!   image's EC header with its CRC;
//! - only the bytes up to a PEB's last non-erased page are programmed;
//! - every good PEB after the image gets a fresh EC header, so UBI attaches
//!   the whole device without formatting it again.
//!
//! The headers, the reserves and the image reader are `gaia_formats::ubi`'s.

use std::io::{Read as IoRead, Seek, SeekFrom};

use gaia_formats::ubi::{
    DEFAULT_BAD_PEBS_PER_1024, EcHeader, HEADER_SIZE, Image, MAX_ERASE_COUNTER, bad_peb_reserve,
    programmed_len,
};

/// A raw NAND flash, addressed by eraseblock: the SPI-NAND behind a
/// Rockchip loader or the K230 burner, or a test's RAM copy.
pub trait RawNand {
    fn peb_count(&self) -> u32;
    fn peb_size(&self) -> u32;
    fn page_size(&self) -> u32;
    fn is_bad(&mut self, peb: u32) -> Result<bool, String>;
    fn read(&mut self, peb: u32, offset: u32, buffer: &mut [u8]) -> Result<(), String>;
    fn erase(&mut self, peb: u32) -> Result<(), String>;
    /// Programs whole pages from `offset` (a page boundary).
    fn program(&mut self, peb: u32, offset: u32, data: &[u8]) -> Result<(), String>;
    fn mark_bad(&mut self, peb: u32) -> Result<(), String>;
}

/// What a format did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatReport {
    /// Where each image PEB went.
    pub placed: Vec<u32>,
    /// Bad before the format.
    pub bad: Vec<u32>,
    /// Marked bad by this format (erase or program failed).
    pub marked_bad: Vec<u32>,
    /// The counter given to PEBs without a readable one.
    pub mean_erase_counter: u64,
    /// Good PEBs after the image, given empty EC headers.
    pub empty_pebs: u32,
}

/// Why a format is refused or stopped: the message, for the operator.
#[derive(Debug, PartialEq, Eq)]
pub struct FormatError(pub String);

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

fn fail<T>(message: impl Into<String>) -> Result<T, FormatError> {
    Err(FormatError(message.into()))
}

/// Writes `image` (read from `source`, described by `info`) to `nand`.
/// `progress` hears (PEBs done, PEBs to do) over the whole device.
pub fn format(
    nand: &mut dyn RawNand,
    source: &mut (impl IoRead + Seek),
    info: &Image,
    verify: bool,
    progress: &mut dyn FnMut(u32, u32),
) -> Result<FormatReport, FormatError> {
    let layout = info.layout;
    if nand.peb_size() != layout.peb_size {
        return fail(format!(
            "the image has {}-byte PEBs; the flash's eraseblocks are {} bytes",
            layout.peb_size,
            nand.peb_size()
        ));
    }
    let page = nand.page_size() as usize;
    if page == 0 || !(layout.peb_size as usize).is_multiple_of(page) {
        return fail(format!("the flash reports a {page}-byte page"));
    }
    let total = nand.peb_count();
    let mut report = FormatReport::default();

    // Scan: bad blocks and the counters the flash already has.
    let mut counters: Vec<Option<u64>> = Vec::with_capacity(total as usize);
    let mut header = [0u8; HEADER_SIZE];
    for peb in 0..total {
        if nand.is_bad(peb).map_err(FormatError)? {
            report.bad.push(peb);
            counters.push(None);
            continue;
        }
        nand.read(peb, 0, &mut header).map_err(FormatError)?;
        counters.push(match EcHeader::decode(&header) {
            Ok(Some(ec)) if ec.erase_counter <= MAX_ERASE_COUNTER => Some(ec.erase_counter),
            _ => None,
        });
    }
    let known: Vec<u64> = counters.iter().flatten().copied().collect();
    report.mean_erase_counter = if known.is_empty() {
        0
    } else {
        known.iter().sum::<u64>() / known.len() as u64
    };
    let good = total as usize - report.bad.len();
    if good < info.pebs as usize {
        return fail(format!(
            "the image needs {} PEBs; the flash has {good} good ones",
            info.pebs
        ));
    }
    // UBI refuses to attach when the volumes' reservations, its own and
    // the bad-block reserve (20 per 1024 PEBs, Linux's default) don't fit.
    let needed = info.reserved_pebs()
        + bad_peb_reserve(u64::from(total), u64::from(DEFAULT_BAD_PEBS_PER_1024));
    if needed > good as u64 {
        return fail(format!(
            "the image's volumes and UBI's reserves need {needed} eraseblocks; the flash has {good} good ones"
        ));
    }
    let next_counter = |peb: u32, counters: &[Option<u64>], mean: u64| {
        counters[peb as usize].map_or(mean, |counter| counter + 1)
    };

    let mut buffer = vec![0u8; layout.peb_size as usize];
    let mut readback = vec![0u8; layout.peb_size as usize];
    let mut peb = 0u32;
    let mut done = 0u32;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| FormatError(format!("the image: {error}")))?;
    for index in 0..info.pebs {
        source
            .read_exact(&mut buffer)
            .map_err(|error| FormatError(format!("image PEB {index}: {error}")))?;
        let Ok(Some(mut ec)) = EcHeader::decode(&buffer) else {
            return fail(format!("image PEB {index} has no EC header"));
        };
        loop {
            if peb >= total {
                return fail(format!(
                    "ran out of good eraseblocks at image PEB {index} ({} went bad while writing)",
                    report.marked_bad.len()
                ));
            }
            if counters[peb as usize].is_none() && report.bad.contains(&peb) {
                peb += 1;
                continue;
            }
            ec.erase_counter = next_counter(peb, &counters, report.mean_erase_counter);
            buffer[..HEADER_SIZE].copy_from_slice(&ec.encode());
            let length = programmed_len(&buffer, page);
            let written = nand.erase(peb).and_then(|()| {
                nand.program(peb, 0, &buffer[..length])?;
                if verify {
                    nand.read(peb, 0, &mut readback[..length])?;
                    if readback[..length] != buffer[..length] {
                        return Err(format!("eraseblock {peb} read back different"));
                    }
                }
                Ok(())
            });
            match written {
                Ok(()) => {
                    report.placed.push(peb);
                    peb += 1;
                    break;
                }
                Err(_) => {
                    nand.mark_bad(peb).map_err(FormatError)?;
                    report.marked_bad.push(peb);
                    peb += 1;
                }
            }
        }
        done += 1;
        progress(done, total - report.bad.len() as u32);
    }

    // The rest: erased, with an EC header each.
    let mut empty = vec![0xffu8; page];
    for rest in peb..total {
        if report.bad.contains(&rest) {
            continue;
        }
        let ec = EcHeader {
            erase_counter: next_counter(rest, &counters, report.mean_erase_counter),
            vid_header_offset: layout.vid_header_offset,
            data_offset: layout.data_offset,
            image_seq: info.image_seq,
        };
        empty[..HEADER_SIZE].copy_from_slice(&ec.encode());
        match nand
            .erase(rest)
            .and_then(|()| nand.program(rest, 0, &empty))
        {
            Ok(()) => report.empty_pebs += 1,
            Err(_) => {
                nand.mark_bad(rest).map_err(FormatError)?;
                report.marked_bad.push(rest);
            }
        }
        done += 1;
        progress(done, total - report.bad.len() as u32);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_image;
    use crate::test_images::sample;

    /// A RAM flash that remembers what was programmed where.
    pub struct RamNand {
        pub pebs: Vec<Vec<u8>>,
        pub bad: Vec<bool>,
        /// Eraseblocks whose next program fails.
        pub failing: Vec<u32>,
        pub programmed: Vec<(u32, usize)>,
        page: u32,
    }

    impl RamNand {
        fn new(count: u32, peb_size: u32, page: u32) -> Self {
            Self {
                pebs: vec![vec![0xff; peb_size as usize]; count as usize],
                bad: vec![false; count as usize],
                failing: Vec::new(),
                programmed: Vec::new(),
                page,
            }
        }
    }

    impl RawNand for RamNand {
        fn peb_count(&self) -> u32 {
            self.pebs.len() as u32
        }
        fn peb_size(&self) -> u32 {
            self.pebs[0].len() as u32
        }
        fn page_size(&self) -> u32 {
            self.page
        }
        fn is_bad(&mut self, peb: u32) -> Result<bool, String> {
            Ok(self.bad[peb as usize])
        }
        fn read(&mut self, peb: u32, offset: u32, buffer: &mut [u8]) -> Result<(), String> {
            let at = offset as usize;
            buffer.copy_from_slice(&self.pebs[peb as usize][at..at + buffer.len()]);
            Ok(())
        }
        fn erase(&mut self, peb: u32) -> Result<(), String> {
            self.pebs[peb as usize].fill(0xff);
            Ok(())
        }
        fn program(&mut self, peb: u32, offset: u32, data: &[u8]) -> Result<(), String> {
            assert_eq!(offset % self.page, 0);
            assert_eq!(data.len() % self.page as usize, 0, "whole pages only");
            if let Some(at) = self.failing.iter().position(|failing| *failing == peb) {
                self.failing.remove(at);
                return Err("program failed".into());
            }
            let block = &mut self.pebs[peb as usize];
            for (cell, byte) in block[offset as usize..].iter_mut().zip(data) {
                *cell &= byte; // NAND only clears bits
            }
            self.programmed.push((peb, data.len()));
            Ok(())
        }
        fn mark_bad(&mut self, peb: u32) -> Result<(), String> {
            self.bad[peb as usize] = true;
            Ok(())
        }
    }

    fn counter(nand: &RamNand, peb: u32) -> u64 {
        match EcHeader::decode(&nand.pebs[peb as usize]) {
            Ok(Some(ec)) => ec.erase_counter,
            other => panic!("PEB {peb}: {other:?}"),
        }
    }

    #[test]
    fn bad_blocks_counters_and_trailing_pages() {
        let rootfs = vec![0x5au8; 20_000];
        let (_, bytes) = sample(&rootfs);
        let info = read_image(&mut std::io::Cursor::new(&bytes), 16 * 1024).unwrap();
        let mut nand = RamNand::new(16, 16 * 1024, 512);
        nand.bad[1] = true;
        // PEB 3 had a counter of 41 from an earlier UBI; the rest none.
        let old = EcHeader {
            erase_counter: 41,
            vid_header_offset: 512,
            data_offset: 1024,
            image_seq: 9,
        };
        nand.pebs[3][..64].copy_from_slice(&old.encode());
        nand.failing.push(4);

        let mut seen = Vec::new();
        let report = format(
            &mut nand,
            &mut std::io::Cursor::new(&bytes),
            &info,
            true,
            &mut |done, of| seen.push((done, of)),
        )
        .unwrap();

        assert_eq!(report.bad, [1]);
        assert_eq!(report.marked_bad, [4]);
        assert_eq!(report.placed, [0, 2, 3, 5]);
        assert_eq!(report.mean_erase_counter, 41);
        assert_eq!(report.empty_pebs, 10); // 6 to 15
        assert_eq!(seen.last(), Some(&(14, 15)));
        assert_eq!(counter(&nand, 3), 42, "its own counter plus one");
        assert_eq!(counter(&nand, 0), 41, "the mean where it had none");
        // Each image PEB landed whole (headers rewritten, data the same).
        for (index, peb) in report.placed.iter().enumerate() {
            let at = index * 16 * 1024;
            assert_eq!(
                nand.pebs[*peb as usize][64..],
                bytes[at + 64..at + 16 * 1024],
                "image PEB {index}"
            );
        }
        // rootfs_a's second LEB holds 20000 - 15360 = 4640 bytes from 1024:
        // 5664 bytes, so 12 pages are programmed, not 32.
        assert!(nand.programmed.contains(&(5, 12 * 512)));
        // An empty PEB carries only its EC header page.
        assert!(nand.programmed.contains(&(9, 512)));
        assert!(
            matches!(EcHeader::decode(&nand.pebs[9]), Ok(Some(ec)) if ec.image_seq == info.image_seq)
        );
    }

    #[test]
    fn a_flash_too_small_or_of_another_geometry_is_refused() {
        let (_, bytes) = sample(&[1; 100]);
        let info = read_image(&mut std::io::Cursor::new(&bytes), 16 * 1024).unwrap();
        // Three PEBs written, but 13 reserved: rootfs_a/b 4 each, data 1,
        // layout 2, UBI's own 2; plus 1 for bad blocks.
        let mut nand = RamNand::new(14, 16 * 1024, 512);
        nand.bad[0] = true;
        let error = format(
            &mut nand,
            &mut std::io::Cursor::new(&bytes),
            &info,
            false,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(error.0.contains("need 14 eraseblocks"), "{error}");
        let mut nand = RamNand::new(3, 16 * 1024, 512);
        nand.bad[0] = true;
        let error = format(
            &mut nand,
            &mut std::io::Cursor::new(&bytes),
            &info,
            false,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(error.0.contains("needs 3 PEBs"), "{error}");
        let mut nand = RamNand::new(4, 128 * 1024, 2048);
        let error = format(
            &mut nand,
            &mut std::io::Cursor::new(&bytes),
            &info,
            false,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(error.0.contains("eraseblocks"), "{error}");
    }
}
