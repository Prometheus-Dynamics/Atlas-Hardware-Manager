//! Writing in loader mode: a whole disk image from an LBA (rkdeveloptool
//! `wl`), read back and compared by SHA-256; the idblock from a loader's
//! FlashHead entries (`ul`); and a UBI image onto SPI-NAND through the LBA
//! commands, for atlas-ubi's ubiformat-style writer.

use std::io::Read;

use sha2::{Digest, Sha256};

use crate::rockchip::loader::Loader;
use crate::rockchip::rockusb::{Rockusb, SECTOR, SECTORS_PER_TRANSFER};
use crate::transport::UsbError;
use crate::{Phase, Progress, hex};

const CHUNK: usize = SECTORS_PER_TRANSFER as usize * SECTOR;

/// What a write put on the board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// Bytes written, the last sector padded with zeros.
    pub bytes: u64,
    /// SHA-256 of those bytes, as read back when verified.
    pub sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    Usb(UsbError),
    Image(String),
    Cancelled,
    /// The board reads back other bytes than were written.
    Mismatch {
        written: String,
        read: String,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usb(error) => error.fmt(f),
            Self::Image(message) => write!(f, "reading the image: {message}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Mismatch { written, read } => write!(
                f,
                "the board reads back {read}, not the {written} written: the storage may be failing"
            ),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<UsbError> for WriteError {
    fn from(error: UsbError) -> Self {
        Self::Usb(error)
    }
}

fn last_lba(start: u32, bytes: u64) -> Result<(), WriteError> {
    let sectors = bytes.div_ceil(SECTOR as u64);
    if u64::from(start) + sectors > u64::from(u32::MAX) {
        return Err(WriteError::Image(format!(
            "{bytes} bytes from LBA {start} is past what 32-bit LBAs address (2 TiB)"
        )));
    }
    Ok(())
}

/// Writes `size` bytes of `image` from `start_lba`, then (with `verify`)
/// reads them back. `cancelled` is checked between transfers.
pub async fn write_image(
    device: &mut Rockusb<'_>,
    image: &mut (dyn Read + Send),
    size: u64,
    start_lba: u32,
    verify: bool,
    progress: Progress<'_>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Written, WriteError> {
    last_lba(start_lba, size)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut lba = start_lba;
    while done < size {
        if cancelled() {
            return Err(WriteError::Cancelled);
        }
        let take = (size - done).min(CHUNK as u64) as usize;
        image
            .read_exact(&mut buffer[..take])
            .map_err(|error| WriteError::Image(error.to_string()))?;
        let padded = take.div_ceil(SECTOR) * SECTOR;
        buffer[take..padded].fill(0);
        hasher.update(&buffer[..padded]);
        device.write_lba(lba, &buffer[..padded]).await?;
        lba += (padded / SECTOR) as u32;
        done += take as u64;
        progress(Phase::Writing, done, size);
    }
    let written = hex(&hasher.finalize());
    let padded_total = size.div_ceil(SECTOR as u64) * SECTOR as u64;
    if verify {
        let read = read_hash(device, start_lba, padded_total, progress, cancelled).await?;
        if read != written {
            return Err(WriteError::Mismatch { written, read });
        }
    }
    Ok(Written {
        bytes: padded_total,
        sha256: written,
    })
}

/// SHA-256 of `bytes` (whole sectors) read from `start_lba`.
pub async fn read_hash(
    device: &mut Rockusb<'_>,
    start_lba: u32,
    bytes: u64,
    progress: Progress<'_>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<String, WriteError> {
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut lba = start_lba;
    while done < bytes {
        if cancelled() {
            return Err(WriteError::Cancelled);
        }
        let take = (bytes - done).min(CHUNK as u64);
        let count = (take / SECTOR as u64) as u16;
        let data = device.read_lba(lba, count).await?;
        hasher.update(&data);
        lba += u32::from(count);
        done += take;
        progress(Phase::Verifying, done, bytes);
    }
    Ok(hex(&hasher.finalize()))
}

/// The idblock of a loader with a FlashHead entry (RK3568 on: RK3588,
/// RK3576, RV1106 loaders carry one): FlashHead, FlashData, FlashBoot one
/// after another, each padded to 2 KiB with zeros (rkdeveloptool sizes them
/// as `ALIGN(size, 2048) / 512` sectors). Written at LBA 64. Loaders
/// without FlashHead need rkdeveloptool's older RC4'd layout, which no
/// board Atlas flashes uses, so they are refused.
pub fn idblock(loader: &Loader, file: &[u8]) -> Result<Vec<u8>, String> {
    let entry = |name: &str| {
        loader
            .loader_entry(name)
            .ok_or_else(|| format!("the loader has no {name} entry"))
    };
    let head = loader.loader_entry("FlashHead").ok_or(
        "the loader has no FlashHead entry: its chip uses the old idblock layout, which Atlas doesn't write",
    )?;
    let (data, boot) = (entry("FlashData")?, entry("FlashBoot")?);
    let mut block = Vec::new();
    let mut sectors = Vec::new();
    for part in [head, data, boot] {
        let start = block.len();
        block.extend_from_slice(part.data(file));
        block.resize(block.len().div_ceil(2048) * 2048, 0);
        sectors.push(((start / 512) as u32, ((block.len() - start) / 512) as u32));
    }
    check_rkns(head.data(file), &sectors[1..])?;
    Ok(block)
}

/// When FlashHead is the RKNS header (RK3568 on), its image table must put
/// FlashData and FlashBoot where the idblock does: (first sector, sectors)
/// each. A head that isn't RKNS isn't checked.
fn check_rkns(head: &[u8], parts: &[(u32, u32)]) -> Result<(), String> {
    let header = crate::rockchip::rc4::rc4_blocks(&head[..head.len().min(512)]);
    if header.get(..4) != Some(b"RKNS".as_slice()) {
        return Ok(());
    }
    for (index, (start, count)) in parts.iter().enumerate() {
        let at = 120 + 88 * index;
        let field =
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
        let (offset, sectors) = (field & 0xffff, field >> 16);
        if (offset, sectors) != (*start, *count) {
            return Err(format!(
                "the loader's FlashHead puts image {index} at sector {offset} for {sectors} sectors, but its entries make it sector {start} for {count}: the loader is inconsistent"
            ));
        }
    }
    Ok(())
}

/// The LBA where the idblock goes on eMMC and SD.
pub const IDBLOCK_LBA: u32 = 64;

/// SPI-NAND seen through loader-mode LBAs, for atlas-ubi's writer: one
/// eraseblock is `peb_size / 512` sectors from `base_lba`. Whether the
/// usbplug maps LBAs onto NAND pages directly or through its own bad-block
/// handling isn't in a source read (UNCONFIRMED), so no block reads as bad
/// here; a failing erase or write still gets the block skipped.
pub struct LbaNand<'d, 'a> {
    pub device: &'d mut Rockusb<'a>,
    pub runtime: tokio::runtime::Handle,
    pub base_lba: u32,
    pub pebs: u32,
    pub peb_size: u32,
    pub page_size: u32,
}

impl LbaNand<'_, '_> {
    fn lba(&self, peb: u32, offset: u32) -> u32 {
        self.base_lba + (peb * (self.peb_size / SECTOR as u32)) + offset / SECTOR as u32
    }
}

impl atlas_ubi::RawNand for LbaNand<'_, '_> {
    fn peb_count(&self) -> u32 {
        self.pebs
    }
    fn peb_size(&self) -> u32 {
        self.peb_size
    }
    fn page_size(&self) -> u32 {
        self.page_size
    }
    fn is_bad(&mut self, _peb: u32) -> Result<bool, String> {
        Ok(false)
    }
    fn read(&mut self, peb: u32, offset: u32, buffer: &mut [u8]) -> Result<(), String> {
        // Whole sectors around the range.
        let first = offset / SECTOR as u32 * SECTOR as u32;
        let end = (offset as usize + buffer.len()).div_ceil(SECTOR) * SECTOR;
        let mut data = Vec::with_capacity(end - first as usize);
        let mut lba = self.lba(peb, first);
        let mut left = (end - first as usize) / SECTOR;
        while left > 0 {
            let count = left.min(usize::from(SECTORS_PER_TRANSFER)) as u16;
            let chunk = self
                .runtime
                .block_on(self.device.read_lba(lba, count))
                .map_err(|error| error.to_string())?;
            data.extend(chunk);
            lba += u32::from(count);
            left -= usize::from(count);
        }
        let skip = (offset - first) as usize;
        buffer.copy_from_slice(&data[skip..skip + buffer.len()]);
        Ok(())
    }
    fn erase(&mut self, peb: u32) -> Result<(), String> {
        let lba = self.lba(peb, 0);
        let count = (self.peb_size / SECTOR as u32) as u16;
        self.runtime
            .block_on(self.device.erase_lba(lba, count))
            .map_err(|error| error.to_string())
    }
    fn program(&mut self, peb: u32, offset: u32, data: &[u8]) -> Result<(), String> {
        let mut lba = self.lba(peb, offset);
        for chunk in data.chunks(CHUNK) {
            self.runtime
                .block_on(self.device.write_lba(lba, chunk))
                .map_err(|error| error.to_string())?;
            lba += (chunk.len() / SECTOR) as u32;
        }
        Ok(())
    }
    fn mark_bad(&mut self, _peb: u32) -> Result<(), String> {
        // No bad-block command is confirmed; the block is skipped this time.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rockchip::crc::LOADER_POLYNOMIALS;
    use crate::rockchip::loader::{EntryKind, build_for_test};
    use crate::rockchip::rockusb::tests::csw;
    use crate::transport::{Script, Step};

    fn cbw_write(tag: u32, lba: u32, bytes: usize) -> Vec<u8> {
        let mut cbw = b"USBC".to_vec();
        cbw.extend_from_slice(&tag.to_le_bytes());
        cbw.extend_from_slice(&(bytes as u32).to_le_bytes());
        cbw.extend_from_slice(&[0x00, 0x00, 0x0a, 0x15, 0x00]);
        cbw.extend_from_slice(&lba.to_be_bytes());
        cbw.push(0);
        cbw.extend_from_slice(&((bytes / 512) as u16).to_be_bytes());
        cbw.resize(31, 0);
        cbw
    }

    fn cbw_read(tag: u32, lba: u32, bytes: usize) -> Vec<u8> {
        let mut cbw = cbw_write(tag, lba, bytes);
        cbw[12] = 0x80;
        cbw[15] = 0x14;
        cbw
    }

    #[tokio::test]
    async fn an_image_is_written_in_64k_transfers_padded_and_verified() {
        // 64 KiB + 100 bytes: two writes, the second one sector, padded.
        let image: Vec<u8> = (0..CHUNK + 100).map(|i| (i % 253) as u8).collect();
        let mut tail = image[CHUNK..].to_vec();
        tail.resize(512, 0);
        let mut script = Script::new(vec![
            Step::BulkOut(cbw_write(1, 0, CHUNK)),
            Step::BulkOut(image[..CHUNK].to_vec()),
            Step::BulkIn {
                max: 13,
                reply: csw(1, 0),
            },
            Step::BulkOut(cbw_write(2, 128, 512)),
            Step::BulkOut(tail.clone()),
            Step::BulkIn {
                max: 13,
                reply: csw(2, 0),
            },
            Step::BulkOut(cbw_read(3, 0, CHUNK)),
            Step::BulkIn {
                max: CHUNK,
                reply: image[..CHUNK].to_vec(),
            },
            Step::BulkIn {
                max: 13,
                reply: csw(3, 0),
            },
            Step::BulkOut(cbw_read(4, 128, 512)),
            Step::BulkIn {
                max: 512,
                reply: tail.clone(),
            },
            Step::BulkIn {
                max: 13,
                reply: csw(4, 0),
            },
        ]);
        let mut device = Rockusb::new(&mut script);
        let mut seen = Vec::new();
        let written = write_image(
            &mut device,
            &mut &image[..],
            image.len() as u64,
            0,
            true,
            &mut |phase, done, of| seen.push((phase, done, of)),
            &|| false,
        )
        .await
        .unwrap();
        let mut padded = image.clone();
        padded.resize(CHUNK + 512, 0);
        assert_eq!(written.bytes, padded.len() as u64);
        assert_eq!(written.sha256, hex(&Sha256::digest(&padded)));
        assert_eq!(
            seen.last(),
            Some(&(Phase::Verifying, padded.len() as u64, padded.len() as u64))
        );
        assert!(script.finished());
    }

    #[tokio::test]
    async fn a_board_reading_back_other_bytes_fails_the_write() {
        let image = vec![1u8; 512];
        let mut script = Script::new(vec![
            Step::BulkOut(cbw_write(1, 64, 512)),
            Step::BulkOut(image.clone()),
            Step::BulkIn {
                max: 13,
                reply: csw(1, 0),
            },
            Step::BulkOut(cbw_read(2, 64, 512)),
            Step::BulkIn {
                max: 512,
                reply: vec![0u8; 512],
            },
            Step::BulkIn {
                max: 13,
                reply: csw(2, 0),
            },
        ]);
        let mut device = Rockusb::new(&mut script);
        let error = write_image(
            &mut device,
            &mut &image[..],
            512,
            64,
            true,
            &mut |_, _, _| {},
            &|| false,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, WriteError::Mismatch { .. }), "{error}");
        let mut script = Script::new(Vec::new());
        let mut device = Rockusb::new(&mut script);
        let error = write_image(
            &mut device,
            &mut &image[..],
            512,
            0,
            false,
            &mut |_, _, _| {},
            &|| true,
        )
        .await
        .unwrap_err();
        assert_eq!(error, WriteError::Cancelled);
    }

    fn rkns_head(parts: &[(u32, u32)]) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..4].copy_from_slice(b"RKNS");
        for (index, (start, count)) in parts.iter().enumerate() {
            let at = 120 + 88 * index;
            header[at..at + 4].copy_from_slice(&((count << 16) | start).to_le_bytes());
        }
        let mut head = crate::rockchip::rc4::rc4(&header);
        head.resize(2048, 0);
        head
    }

    #[test]
    fn an_rkns_head_must_agree_with_the_idblock() {
        let loader_with = |head: &[u8]| {
            build_for_test(
                &[
                    (EntryKind::Ddr, "ddr", &[1; 8], 0),
                    (EntryKind::Usbplug, "usbplug", &[2; 8], 0),
                    (EntryKind::Loader, "FlashHead", head, 0),
                    (EntryKind::Loader, "FlashData", &[4; 4096], 0),
                    (EntryKind::Loader, "FlashBoot", &[5; 6144], 0),
                ],
                LOADER_POLYNOMIALS[0],
            )
        };
        // FlashData at sector 4 for 8, FlashBoot at 12 for 12: as built.
        let good = loader_with(&rkns_head(&[(4, 8), (12, 12)]));
        let loader = Loader::parse(&good).unwrap();
        assert_eq!(idblock(&loader, &good).unwrap().len(), 2048 + 4096 + 6144);
        let bad = loader_with(&rkns_head(&[(4, 8), (13, 12)]));
        let loader = Loader::parse(&bad).unwrap();
        assert!(idblock(&loader, &bad).unwrap_err().contains("inconsistent"));
    }

    #[test]
    fn the_idblock_is_head_data_boot_each_on_2k() {
        let file = build_for_test(
            &[
                (EntryKind::Ddr, "ddr", &[1; 8], 0),
                (EntryKind::Usbplug, "usbplug", &[2; 8], 0),
                (EntryKind::Loader, "FlashHead", &[3; 2048], 0),
                (EntryKind::Loader, "FlashData", &[4; 100], 0),
                (EntryKind::Loader, "FlashBoot", &[5; 3000], 0),
            ],
            LOADER_POLYNOMIALS[0],
        );
        let loader = Loader::parse(&file).unwrap();
        let block = idblock(&loader, &file).unwrap();
        assert_eq!(block.len(), 2048 + 2048 + 4096);
        assert_eq!(block[2048], 4);
        assert_eq!(block[2048 + 100], 0);
        assert_eq!(block[4096], 5);
        let legacy = build_for_test(
            &[
                (EntryKind::Ddr, "ddr", &[1; 8], 0),
                (EntryKind::Usbplug, "usbplug", &[2; 8], 0),
                (EntryKind::Loader, "FlashData", &[4; 100], 0),
                (EntryKind::Loader, "FlashBoot", &[5; 300], 0),
            ],
            LOADER_POLYNOMIALS[0],
        );
        let loader = Loader::parse(&legacy).unwrap();
        assert!(idblock(&loader, &legacy).unwrap_err().contains("FlashHead"));
    }
}
