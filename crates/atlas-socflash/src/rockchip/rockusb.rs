//! Loader mode (the usbplug maskrom ran, or a loader in flash): commands
//! over bulk as 31-byte CBWs ("USBC") answered by 13-byte CSWs ("USBS"),
//! rkdeveloptool RKComm.h/.cpp. The CBW's integers are little-endian; the
//! command block's LBA and sector count are big-endian. Sectors are 512
//! bytes.

use crate::transport::{UsbError, UsbIo};

pub const SECTOR: usize = 512;
/// Sectors per read or write command, as rkdeveloptool moves them.
pub const SECTORS_PER_TRANSFER: u16 = 128;

const CBW_SIGNATURE: [u8; 4] = *b"USBC";
const CSW_SIGNATURE: [u8; 4] = *b"USBS";
const CBW_LEN: usize = 31;
const CSW_LEN: usize = 13;

/// Opcodes (rkdeveloptool `USB_OPERATION_CODE`).
pub mod op {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const READ_FLASH_ID: u8 = 0x01;
    pub const READ_LBA: u8 = 0x14;
    pub const WRITE_LBA: u8 = 0x15;
    pub const READ_FLASH_INFO: u8 = 0x1a;
    pub const READ_CHIP_INFO: u8 = 0x1b;
    pub const ERASE_LBA: u8 = 0x25;
    pub const CHANGE_STORAGE: u8 = 0x2a;
    pub const READ_STORAGE: u8 = 0x2b;
    pub const READ_CAPABILITY: u8 = 0xaa;
    pub const DEVICE_RESET: u8 = 0xff;
}

/// `DEVICE_RESET` subcodes (RKComm.h `RESET_SUBCODE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    /// Restart into whatever the flash now holds.
    Normal = 0,
    Msc = 1,
    PowerOff = 2,
    Maskrom = 3,
    Disconnect = 4,
}

/// Storage ids for `CHANGE_STORAGE`, from rkdeveloptool's `cs` usage
/// (1 eMMC, 2 SD, 9 SPI-NOR). SPI-NAND's id isn't in a source read
/// (UNCONFIRMED); NAND boards' usbplugs start on their NAND.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    Emmc = 1,
    Sd = 2,
    SpiNor = 9,
}

/// A command block: the opcode, its subcode, and for the long (10-byte)
/// form an LBA and a sector count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cb {
    pub opcode: u8,
    pub sub: u8,
    pub lba: u32,
    pub count: u16,
    pub long: bool,
}

impl Cb {
    pub fn short(opcode: u8, sub: u8) -> Self {
        Self {
            opcode,
            sub,
            lba: 0,
            count: 0,
            long: false,
        }
    }

    pub fn lba(opcode: u8, lba: u32, count: u16) -> Self {
        Self {
            opcode,
            sub: 0,
            lba,
            count,
            long: true,
        }
    }
}

/// The 31-byte CBW of `cb` with `transfer` data bytes, in or out.
pub fn cbw(tag: u32, transfer: u32, input: bool, cb: Cb) -> [u8; CBW_LEN] {
    let mut cbw = [0u8; CBW_LEN];
    cbw[0..4].copy_from_slice(&CBW_SIGNATURE);
    cbw[4..8].copy_from_slice(&tag.to_le_bytes());
    cbw[8..12].copy_from_slice(&transfer.to_le_bytes());
    cbw[12] = if input { 0x80 } else { 0x00 };
    cbw[14] = if cb.long { 0x0a } else { 0x06 };
    cbw[15] = cb.opcode;
    cbw[16] = cb.sub;
    cbw[17..21].copy_from_slice(&cb.lba.to_be_bytes());
    cbw[22..24].copy_from_slice(&cb.count.to_be_bytes());
    cbw
}

/// A loader-mode device.
pub struct Rockusb<'a> {
    io: &'a mut dyn UsbIo,
    tag: u32,
}

/// What `READ_FLASH_INFO` says. Its layout isn't in a source read beyond
/// the size (UNCONFIRMED): the first u32 is the flash size in sectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashInfo {
    pub sectors: u32,
    pub raw: Vec<u8>,
}

impl FlashInfo {
    pub fn bytes(&self) -> u64 {
        u64::from(self.sectors) * SECTOR as u64
    }
}

impl<'a> Rockusb<'a> {
    pub fn new(io: &'a mut dyn UsbIo) -> Self {
        Self { io, tag: 0 }
    }

    /// One command: its CBW, its data (out or in), its CSW.
    async fn command(
        &mut self,
        cb: Cb,
        out: Option<&[u8]>,
        input: Option<usize>,
    ) -> Result<Vec<u8>, UsbError> {
        self.tag = self.tag.wrapping_add(1);
        let tag = self.tag;
        let transfer = out.map_or(input.unwrap_or(0), <[u8]>::len) as u32;
        let wire = cbw(tag, transfer, input.is_some(), cb);
        let Cb {
            opcode, lba, count, ..
        } = cb;
        self.io.bulk_out(&wire).await?;
        let mut reply = Vec::new();
        if let Some(data) = out {
            self.io.bulk_out(data).await?;
        }
        if let Some(length) = input {
            reply = self.io.bulk_in(length).await?;
        }
        let csw = self.io.bulk_in(CSW_LEN).await?;
        if csw.len() != CSW_LEN || csw[0..4] != CSW_SIGNATURE {
            return Err(UsbError::Protocol(format!(
                "command {opcode:#04x}: expected a CSW, got {} bytes",
                csw.len()
            )));
        }
        let echoed = u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]);
        if echoed != tag {
            return Err(UsbError::Protocol(format!(
                "command {opcode:#04x}: CSW tag {echoed}, sent {tag}"
            )));
        }
        // The residue (8..12) is skipped: U-Boot's rockusb writes it
        // big-endian, rkdeveloptool reads it native.
        if csw[12] != 0 {
            return Err(UsbError::Protocol(format!(
                "command {opcode:#04x} (LBA {lba}, {count} sectors) failed on the board (status {})",
                csw[12]
            )));
        }
        Ok(reply)
    }

    pub async fn test_unit_ready(&mut self) -> Result<(), UsbError> {
        self.command(Cb::short(op::TEST_UNIT_READY, 0), None, None)
            .await
            .map(drop)
    }

    pub async fn read_flash_id(&mut self) -> Result<Vec<u8>, UsbError> {
        self.command(Cb::short(op::READ_FLASH_ID, 0), None, Some(5))
            .await
    }

    pub async fn read_flash_info(&mut self) -> Result<FlashInfo, UsbError> {
        let raw = self
            .command(Cb::short(op::READ_FLASH_INFO, 0), None, Some(SECTOR))
            .await?;
        if raw.len() < 4 {
            return Err(UsbError::Protocol(format!(
                "flash info of {} bytes",
                raw.len()
            )));
        }
        Ok(FlashInfo {
            sectors: u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
            raw,
        })
    }

    pub async fn read_chip_info(&mut self) -> Result<Vec<u8>, UsbError> {
        self.command(Cb::short(op::READ_CHIP_INFO, 0), None, Some(16))
            .await
    }

    pub async fn read_capability(&mut self) -> Result<Vec<u8>, UsbError> {
        self.command(Cb::short(op::READ_CAPABILITY, 0), None, Some(8))
            .await
    }

    pub async fn change_storage(&mut self, storage: Storage) -> Result<(), UsbError> {
        self.command(Cb::short(op::CHANGE_STORAGE, storage as u8), None, None)
            .await
            .map(drop)
    }

    /// Writes whole sectors at `lba` (at most [`SECTORS_PER_TRANSFER`]).
    pub async fn write_lba(&mut self, lba: u32, data: &[u8]) -> Result<(), UsbError> {
        let count = sectors(data.len())?;
        self.command(Cb::lba(op::WRITE_LBA, lba, count), Some(data), None)
            .await
            .map(drop)
    }

    pub async fn read_lba(&mut self, lba: u32, count: u16) -> Result<Vec<u8>, UsbError> {
        let data = self
            .command(
                Cb::lba(op::READ_LBA, lba, count),
                None,
                Some(usize::from(count) * SECTOR),
            )
            .await?;
        if data.len() != usize::from(count) * SECTOR {
            return Err(UsbError::Protocol(format!(
                "read {} bytes of {} sectors at LBA {lba}",
                data.len(),
                count
            )));
        }
        Ok(data)
    }

    pub async fn erase_lba(&mut self, lba: u32, count: u16) -> Result<(), UsbError> {
        self.command(Cb::lba(op::ERASE_LBA, lba, count), None, None)
            .await
            .map(drop)
    }

    pub async fn reset(&mut self, how: Reset) -> Result<(), UsbError> {
        match self
            .command(Cb::short(op::DEVICE_RESET, how as u8), None, None)
            .await
        {
            // A board may go before its CSW arrives.
            Err(UsbError::Disconnected) => Ok(()),
            other => other.map(drop),
        }
    }
}

fn sectors(bytes: usize) -> Result<u16, UsbError> {
    if bytes == 0
        || !bytes.is_multiple_of(SECTOR)
        || bytes / SECTOR > usize::from(SECTORS_PER_TRANSFER)
    {
        return Err(UsbError::Protocol(format!(
            "{bytes} bytes isn't 1 to {SECTORS_PER_TRANSFER} whole sectors"
        )));
    }
    Ok((bytes / SECTOR) as u16)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::transport::{Script, Step};

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    pub fn csw(tag: u32, status: u8) -> Vec<u8> {
        let mut csw = b"USBS".to_vec();
        csw.extend_from_slice(&tag.to_le_bytes());
        csw.extend_from_slice(&[0, 0, 0, 0, status]);
        csw
    }

    #[test]
    fn cbws_match_the_spec_vectors() {
        // The research spec's vectors, from rkdeveloptool's layout.
        assert_eq!(
            cbw(1, 5, true, Cb::short(op::READ_FLASH_ID, 0)).to_vec(),
            hex("55534243010000000500000080000601000000000000000000000000000000")
        );
        assert_eq!(
            cbw(2, 2048, false, Cb::lba(op::WRITE_LBA, 0x40, 4)).to_vec(),
            hex("55534243020000000008000000000a15000000004000000400000000000000")
        );
    }

    #[tokio::test]
    async fn a_write_a_read_and_a_failed_command() {
        let data = vec![0xabu8; 1024];
        let mut script = Script::new(vec![
            Step::BulkOut(cbw(1, 1024, false, Cb::lba(op::WRITE_LBA, 64, 2)).to_vec()),
            Step::BulkOut(data.clone()),
            Step::BulkIn {
                max: 13,
                reply: csw(1, 0),
            },
            Step::BulkOut(cbw(2, 1024, true, Cb::lba(op::READ_LBA, 64, 2)).to_vec()),
            Step::BulkIn {
                max: 1024,
                reply: data.clone(),
            },
            Step::BulkIn {
                max: 13,
                reply: csw(2, 0),
            },
            Step::BulkOut(cbw(3, 0, false, Cb::lba(op::ERASE_LBA, 0, 16)).to_vec()),
            Step::BulkIn {
                max: 13,
                reply: csw(3, 1),
            },
            Step::BulkOut(cbw(4, 0, false, Cb::short(op::DEVICE_RESET, 0)).to_vec()),
            Step::Disconnect,
        ]);
        let mut device = Rockusb::new(&mut script);
        device.write_lba(64, &data).await.unwrap();
        assert_eq!(device.read_lba(64, 2).await.unwrap(), data);
        let error = device.erase_lba(0, 16).await.unwrap_err();
        assert!(error.to_string().contains("failed on the board"), "{error}");
        device.reset(Reset::Normal).await.unwrap();
        assert!(script.finished());
    }

    #[tokio::test]
    async fn a_wrong_tag_or_partial_sectors_are_refused() {
        let mut script = Script::new(vec![
            Step::BulkOut(cbw(1, 0, false, Cb::short(op::TEST_UNIT_READY, 0)).to_vec()),
            Step::BulkIn {
                max: 13,
                reply: csw(7, 0),
            },
        ]);
        let mut device = Rockusb::new(&mut script);
        assert!(
            device
                .test_unit_ready()
                .await
                .unwrap_err()
                .to_string()
                .contains("tag")
        );
        assert!(device.write_lba(0, &[0; 100]).await.is_err());
        assert!(device.write_lba(0, &vec![0; 129 * 512]).await.is_err());
    }
}
