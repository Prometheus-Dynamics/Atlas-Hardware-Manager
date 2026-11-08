//! A mock Rockchip board for dry runs. [`Board`] takes the maskrom download,
//! then answers rockusb the way rkdeveloptool's CBW/CSW framing expects
//! (RKComm.cpp), from an in-memory disk. [`walk`] runs the flash Atlas does,
//! in the same order and through the same functions, against that board: a
//! dry run needs no USB device. Its log is the transfers, through
//! [`crate::traced::Traced`].
//!
//! The mock is only as right as the source it was written from; it proves
//! the sequence and the framing, not what a real usbplug answers.

use std::sync::{Arc, Mutex};

use crate::Phase;
use crate::rockchip::maskrom;
use crate::rockchip::rockusb::{self, Reset, Rockusb, SECTOR, Storage, op};
use crate::rockchip::write::{self, Written};
use crate::traced::Traced;
use crate::transport::{UsbError, UsbIo};

const CBW_LEN: usize = 31;
/// The two maskrom stages (wIndex 0x471 then 0x472).
const SEEN_DDR: u8 = 1;
const SEEN_USBPLUG: u8 = 2;

/// A board that boots in maskrom and then serves rockusb from `disk`.
pub struct Board {
    /// The chip number its READ_CHIP_INFO reply carries ("3588").
    chip: String,
    disk: Arc<Mutex<Vec<u8>>>,
    /// Storage ids it has (bit `id` set, as READ_STORAGE's mask reads).
    available: u32,
    /// The storage it now uses, as READ_STORAGE's mask.
    storage: u32,
    stages: u8,
    loader_mode: bool,
    gone: bool,
    tag: u32,
    replies: std::collections::VecDeque<Vec<u8>>,
    /// The byte range a WRITE_LBA's data goes to, once its CBW is in.
    write_to: Option<(usize, usize)>,
}

impl Board {
    /// A board with `sectors` sectors of storage, `chip` its chip number,
    /// and the storage ids `available` (1 eMMC, 2 SD, 9 SPI-NOR). It starts
    /// on the first of them.
    pub fn new(chip: &str, sectors: u32, available: &[Storage]) -> Self {
        let available = available
            .iter()
            .fold(0u32, |mask, id| mask | (1 << *id as u8));
        let first = (0..32).find(|bit| available & (1 << bit) != 0).unwrap_or(1);
        Self {
            chip: chip.to_string(),
            disk: Arc::new(Mutex::new(vec![0u8; sectors as usize * SECTOR])),
            available,
            storage: 1 << first,
            stages: 0,
            loader_mode: false,
            gone: false,
            tag: 0,
            replies: std::collections::VecDeque::new(),
            write_to: None,
        }
    }

    /// The disk, shared: read it after the board has been moved into a walk.
    pub fn disk(&self) -> Arc<Mutex<Vec<u8>>> {
        Arc::clone(&self.disk)
    }

    /// READ_CHIP_INFO's 16 bytes: the chip number reversed, as its reply
    /// reads for rkdeveloptool's chip check (`RKDevice.cpp`, CheckChip).
    fn chip_info(&self) -> Vec<u8> {
        let mut info: Vec<u8> = self.chip.bytes().rev().collect();
        info.resize(16, 0);
        info
    }

    /// READ_FLASH_INFO's reply: 16 KiB blocks and 2 KiB pages, as sectors.
    fn flash_info(&self) -> Vec<u8> {
        let sectors = (self.disk.lock().map_or(0, |disk| disk.len()) / SECTOR) as u32;
        let mut info = Vec::with_capacity(512);
        info.extend_from_slice(&sectors.to_le_bytes());
        info.extend_from_slice(&32u16.to_le_bytes());
        info.push(4);
        info.resize(512, 0);
        info
    }

    fn csw(tag: u32, status: u8) -> Vec<u8> {
        let mut csw = b"USBS".to_vec();
        csw.extend_from_slice(&tag.to_le_bytes());
        csw.extend_from_slice(&[0; 4]);
        csw.push(status);
        csw
    }

    fn command(&mut self, cbw: &[u8]) -> Result<(), UsbError> {
        let tag = u32::from_le_bytes([cbw[4], cbw[5], cbw[6], cbw[7]]);
        let transfer = u32::from_le_bytes([cbw[8], cbw[9], cbw[10], cbw[11]]) as usize;
        let opcode = cbw[15];
        let sub = cbw[16];
        let lba = u32::from_be_bytes([cbw[17], cbw[18], cbw[19], cbw[20]]) as usize;
        let count = usize::from(u16::from_be_bytes([cbw[22], cbw[23]]));
        let disk_len = self.disk.lock().map_or(0, |disk| disk.len());
        let span = lba * SECTOR..(lba + count) * SECTOR;
        let reply = match opcode {
            op::TEST_UNIT_READY => Vec::new(),
            op::READ_FLASH_ID => vec![0; transfer.min(5)],
            op::READ_FLASH_INFO => self.flash_info()[..transfer.min(512)].to_vec(),
            op::READ_CHIP_INFO => self.chip_info()[..transfer.min(16)].to_vec(),
            op::READ_CAPABILITY => vec![0; transfer.min(8)],
            op::READ_STORAGE => self.storage.to_le_bytes().to_vec(),
            op::CHANGE_STORAGE => {
                // Silent when the storage isn't there, as rkdeveloptool's
                // cs warns: the read-back is what tells.
                if self.available & (1 << sub) != 0 {
                    self.storage = 1 << sub;
                }
                Vec::new()
            }
            op::READ_LBA if span.end <= disk_len => {
                let data = self.disk.lock().map(|disk| disk[span].to_vec());
                let data = data.unwrap_or_default();
                self.replies.push_back(data);
                Vec::new()
            }
            op::WRITE_LBA if span.end <= disk_len && transfer == count * SECTOR => {
                self.write_to = Some((lba * SECTOR, transfer));
                return Ok(());
            }
            op::ERASE_LBA if span.end <= disk_len => {
                if let Ok(mut disk) = self.disk.lock() {
                    disk[span].fill(0);
                }
                Vec::new()
            }
            op::DEVICE_RESET => {
                // The board goes off USB; its CSW never comes.
                self.gone = true;
                return Ok(());
            }
            _ => {
                // Refused: a status of 1 and nothing else.
                self.replies.push_back(Self::csw(tag, 1));
                return Ok(());
            }
        };
        if !reply.is_empty() {
            self.replies.push_back(reply);
        }
        self.replies.push_back(Self::csw(tag, 0));
        Ok(())
    }
}

#[async_trait::async_trait]
impl UsbIo for Board {
    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        _data: &[u8],
    ) -> Result<(), UsbError> {
        if self.loader_mode {
            return Err(UsbError::Protocol(
                "a maskrom request after the usbplug ran".into(),
            ));
        }
        if request != maskrom::REQUEST || value != 0 {
            return Err(UsbError::Protocol(format!(
                "request {request:#04x} value {value:#06x} isn't a maskrom download"
            )));
        }
        match index {
            maskrom::STAGE_DDR => self.stages |= SEEN_DDR,
            maskrom::STAGE_USBPLUG => self.stages |= SEEN_USBPLUG,
            other => {
                return Err(UsbError::Protocol(format!(
                    "maskrom stage {other:#06x} isn't 0x0471 or 0x0472"
                )));
            }
        }
        Ok(())
    }

    async fn control_in(
        &mut self,
        request: u8,
        _value: u16,
        _index: u16,
        _length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        Err(UsbError::Protocol(format!(
            "the mock answers no vendor read (request {request:#04x})"
        )))
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        if self.gone {
            return Err(UsbError::Disconnected);
        }
        if let Some((offset, len)) = self.write_to.take() {
            if data.len() != len {
                return Err(UsbError::Protocol(format!(
                    "write data of {} bytes, the CBW said {len}",
                    data.len()
                )));
            }
            if let Ok(mut disk) = self.disk.lock() {
                disk[offset..offset + len].copy_from_slice(data);
            }
            let tag = self.tag;
            self.replies.push_back(Self::csw(tag, 0));
            return Ok(());
        }
        if self.stages == SEEN_DDR | SEEN_USBPLUG {
            // The usbplug runs and the board re-enumerates in loader mode.
            self.loader_mode = true;
        }
        if !self.loader_mode {
            return Err(UsbError::Protocol(
                "a bulk transfer before the usbplug ran".into(),
            ));
        }
        if data.len() != CBW_LEN || data[0..4] != *b"USBC" {
            return Err(UsbError::Protocol(format!(
                "{} bytes that aren't a CBW",
                data.len()
            )));
        }
        self.tag = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        self.command(data)
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        if self.gone {
            return Err(UsbError::Disconnected);
        }
        let Some(mut reply) = self.replies.pop_front() else {
            return Err(UsbError::Protocol(
                "the host reads, but the board has nothing to send".into(),
            ));
        };
        reply.truncate(max);
        Ok(reply)
    }
}

/// Walks a whole flash on `board`: the loader's 471 and 472 entries, the
/// chip check, the storage (read back), the flash info, the image written
/// and read back, then a reset. Returns what was written. Each transfer is
/// logged through [`Traced`], and to `sink` when one is given.
pub async fn walk(
    loader_file: &[u8],
    image: &[u8],
    start_lba: u32,
    storage: Option<Storage>,
    board: Board,
    sink: Option<crate::traced::Sink>,
) -> Result<Written, String> {
    let loader = crate::rockchip::loader::Loader::parse(loader_file).map_err(|e| e.to_string())?;
    let mut traced = Traced::new(Box::new(board), "dry-run");
    if let Some(sink) = sink {
        traced = traced.with_sink(sink);
    }
    let mut io = traced;
    maskrom::download_boot(&mut io, &loader, loader_file)
        .await
        .map_err(|e| e.to_string())?;
    let mut device = Rockusb::new(&mut io);
    device.test_unit_ready().await.map_err(|e| e.to_string())?;
    let chip = device.read_chip_info().await.map_err(|e| e.to_string())?;
    if let Some(number) = loader.chip_number()
        && rockusb::chip_matches(&chip, &format!("RK{number}")) != Some(true)
    {
        return Err(format!("the board's chip reply doesn't read as {number}"));
    }
    if let Some(storage) = storage {
        device
            .change_storage(storage)
            .await
            .map_err(|e| e.to_string())?;
    }
    let info = device.read_flash_info().await.map_err(|e| e.to_string())?;
    if info.bytes() < image.len() as u64 {
        return Err(format!(
            "the image ({} bytes) doesn't fit {} bytes",
            image.len(),
            info.bytes()
        ));
    }
    let mut reader: &[u8] = image;
    let written = write::write_image(
        &mut device,
        &mut reader,
        image.len() as u64,
        start_lba,
        true,
        &mut |_: Phase, _: u64, _: u64| {},
        &|| false,
    )
    .await
    .map_err(|e| e.to_string())?;
    device
        .reset(Reset::Normal)
        .await
        .map_err(|e| e.to_string())?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rockchip::crc::LOADER_POLYNOMIALS;
    use crate::rockchip::loader::{EntryKind, build_for_chip};
    use sha2::{Digest, Sha256};

    /// A loader for RK3588 (its header's chip reads "3588").
    fn loader() -> Vec<u8> {
        build_for_chip(
            b"8853",
            &[
                (EntryKind::Ddr, "ddr", &[1u8; 300], 0),
                (EntryKind::Usbplug, "usbplug", &[2u8; 300], 0),
            ],
            LOADER_POLYNOMIALS[0],
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_whole_flash_walks_and_reads_back() {
        let image: Vec<u8> = (0..3000u32).map(|n| (n % 251) as u8).collect();
        let board = Board::new("3588", 1 << 16, &[Storage::Emmc, Storage::Sd]);
        let disk = board.disk();
        let written = walk(&loader(), &image, 64, Some(Storage::Emmc), board, None)
            .await
            .unwrap();
        assert_eq!(written.bytes, 3072);
        // The write hashes the image padded with zeros to whole sectors.
        let mut padded = image.clone();
        padded.resize(3072, 0);
        assert_eq!(
            written.sha256,
            crate::hex(Sha256::digest(&padded).as_slice())
        );
        let disk = disk.lock().unwrap();
        assert_eq!(&disk[64 * SECTOR..64 * SECTOR + image.len()], &image[..]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_storage_the_board_lacks_is_refused_before_writing() {
        let board = Board::new("3588", 1 << 16, &[Storage::Emmc]);
        let disk = board.disk();
        let error = walk(
            &loader(),
            &[7u8; 512],
            64,
            Some(Storage::SpiNor),
            board,
            None,
        )
        .await
        .unwrap_err();
        assert!(error.contains("isn't available"), "{error}");
        assert!(disk.lock().unwrap().iter().all(|byte| *byte == 0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_chip_the_loader_is_not_for_is_refused() {
        let board = Board::new("3576", 1 << 16, &[Storage::Emmc]);
        let error = walk(&loader(), &[7u8; 512], 64, None, board, None)
            .await
            .unwrap_err();
        assert!(error.contains("chip"), "{error}");
    }
}
