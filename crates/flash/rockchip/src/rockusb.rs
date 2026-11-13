//! Loader mode (the usbplug maskrom ran): commands over bulk as 31-byte CBWs
//! ("USBC") answered by 13-byte CSWs ("USBS"), as rkdeveloptool RKComm.h and
//! RKComm.cpp speak them. The CBW's integers are little-endian.
//!
//! Only the read-only queries are here: test unit ready, flash id, flash
//! info, chip info, capability and storage. Writing, erasing and changing
//! storage are not part of this crate, and nothing here starts an image.

use atlas_usb::{UsbError, UsbIo};

use crate::UsbResult;

const CBW_SIGNATURE: [u8; 4] = *b"USBC";
const CSW_SIGNATURE: [u8; 4] = *b"USBS";
const CBW_LEN: usize = 31;
const CSW_LEN: usize = 13;

/// Opcodes (rkdeveloptool `USB_OPERATION_CODE`).
pub mod op {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const READ_FLASH_ID: u8 = 0x01;
    pub const READ_FLASH_INFO: u8 = 0x1a;
    pub const READ_CHIP_INFO: u8 = 0x1b;
    pub const READ_STORAGE: u8 = 0x2b;
    pub const READ_CAPABILITY: u8 = 0xaa;
}

/// Bytes of `STRUCT_FLASHINFO_CMD`'s fields (the struct's 501 reserved bytes
/// follow them). The command asks for these, and the reply may run to a
/// sector (RKComm.cpp:400).
const FLASH_INFO_FIELDS: usize = 11;
const SECTOR: usize = 512;

/// A command block: the opcode and its subcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cb {
    pub opcode: u8,
    pub sub: u8,
}

/// The 31-byte CBW of `cb` with `transfer` data bytes coming back.
pub fn cbw(tag: u32, transfer: u32, cb: Cb) -> [u8; CBW_LEN] {
    let mut cbw = [0u8; CBW_LEN];
    cbw[0..4].copy_from_slice(&CBW_SIGNATURE);
    cbw[4..8].copy_from_slice(&tag.to_le_bytes());
    cbw[8..12].copy_from_slice(&transfer.to_le_bytes());
    cbw[12] = 0x80; // device to host
    cbw[14] = 0x06; // short command block
    cbw[15] = cb.opcode;
    cbw[16] = cb.sub;
    cbw
}

/// What `READ_FLASH_INFO` says: rkdeveloptool's `STRUCT_FLASHINFO_CMD`,
/// packed. Sizes are in sectors (RKDevice.cpp:308-318 halves them to KiB).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashInfo {
    /// Flash size in sectors (`uiFlashSize`).
    pub sectors: u32,
    /// Block size in sectors (`usBlockSize`).
    pub block_sectors: u16,
    /// Page size in sectors (`bPageSize`).
    pub page_sectors: u8,
    pub ecc_bits: u8,
    pub access_time: u8,
    pub manufacturer: u8,
    pub chip_select: u8,
    pub raw: Vec<u8>,
}

impl FlashInfo {
    /// The fields of a reply of at least 11 bytes.
    pub fn from_reply(raw: Vec<u8>) -> UsbResult<Self> {
        if raw.len() < FLASH_INFO_FIELDS {
            return Err(UsbError::Protocol(format!(
                "flash info of {} bytes; rkdeveloptool's layout needs {FLASH_INFO_FIELDS}",
                raw.len()
            )));
        }
        Ok(Self {
            sectors: u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
            block_sectors: u16::from_le_bytes([raw[4], raw[5]]),
            page_sectors: raw[6],
            ecc_bits: raw[7],
            access_time: raw[8],
            manufacturer: raw[9],
            chip_select: raw[10],
            raw,
        })
    }

    /// The flash size in bytes.
    pub fn bytes(&self) -> u64 {
        u64::from(self.sectors) * SECTOR as u64
    }
}

/// A loader-mode device, driven one command at a time.
pub struct Rockusb<'a> {
    io: &'a mut dyn UsbIo,
    tag: u32,
}

impl std::fmt::Debug for Rockusb<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rockusb")
            .field("tag", &self.tag)
            .finish_non_exhaustive()
    }
}

impl<'a> Rockusb<'a> {
    pub fn new(io: &'a mut dyn UsbIo) -> Self {
        Self { io, tag: 0 }
    }

    /// One command: its CBW, then `input` bytes back if any, then its CSW.
    async fn command(&mut self, cb: Cb, transfer: u32, input: Option<usize>) -> UsbResult<Vec<u8>> {
        self.tag = self.tag.wrapping_add(1);
        let tag = self.tag;
        self.io.bulk_out(&cbw(tag, transfer, cb)).await?;
        let mut reply = Vec::new();
        if let Some(length) = input {
            reply = self.io.bulk_in(length).await?;
        }
        let csw = self.io.bulk_in(CSW_LEN).await?;
        if csw.len() != CSW_LEN || csw[0..4] != CSW_SIGNATURE {
            return Err(UsbError::Protocol(format!(
                "command {:#04x}: expected a CSW, got {} bytes",
                cb.opcode,
                csw.len()
            )));
        }
        let echoed = u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]);
        if echoed != tag {
            return Err(UsbError::Protocol(format!(
                "command {:#04x}: CSW tag {echoed}, sent {tag}",
                cb.opcode
            )));
        }
        // The residue (8..12) is skipped: U-Boot's rockusb writes it
        // big-endian, rkdeveloptool reads it native.
        if csw[12] != 0 {
            return Err(UsbError::Protocol(format!(
                "command {:#04x} failed on the board (status {})",
                cb.opcode, csw[12]
            )));
        }
        Ok(reply)
    }

    pub async fn test_unit_ready(&mut self) -> UsbResult<()> {
        self.command(
            Cb {
                opcode: op::TEST_UNIT_READY,
                sub: 0,
            },
            0,
            None,
        )
        .await
        .map(drop)
    }

    pub async fn read_flash_id(&mut self) -> UsbResult<Vec<u8>> {
        self.command(
            Cb {
                opcode: op::READ_FLASH_ID,
                sub: 0,
            },
            5,
            Some(5),
        )
        .await
    }

    pub async fn read_flash_info(&mut self) -> UsbResult<FlashInfo> {
        let raw = self
            .command(
                Cb {
                    opcode: op::READ_FLASH_INFO,
                    sub: 0,
                },
                FLASH_INFO_FIELDS as u32,
                Some(SECTOR),
            )
            .await?;
        FlashInfo::from_reply(raw)
    }

    pub async fn read_chip_info(&mut self) -> UsbResult<Vec<u8>> {
        self.command(
            Cb {
                opcode: op::READ_CHIP_INFO,
                sub: 0,
            },
            16,
            Some(16),
        )
        .await
    }

    pub async fn read_capability(&mut self) -> UsbResult<Vec<u8>> {
        self.command(
            Cb {
                opcode: op::READ_CAPABILITY,
                sub: 0,
            },
            8,
            Some(8),
        )
        .await
    }

    /// The storage the usbplug now uses: the index of the first bit set in
    /// its 4-byte mask (RKComm.cpp:599-647). `None` when no bit is set.
    pub async fn read_storage(&mut self) -> UsbResult<Option<u8>> {
        let reply = self
            .command(
                Cb {
                    opcode: op::READ_STORAGE,
                    sub: 0,
                },
                4,
                Some(4),
            )
            .await?;
        let [a, b, c, d] = reply[..] else {
            return Err(UsbError::Protocol(format!(
                "storage mask of {} bytes; rkdeveloptool reads 4",
                reply.len()
            )));
        };
        let mask = u32::from_le_bytes([a, b, c, d]);
        Ok((0..32u8).find(|bit| mask & (1 << bit) != 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbw_matches_the_spec_vector() {
        assert_eq!(
            cbw(
                1,
                5,
                Cb {
                    opcode: op::READ_FLASH_ID,
                    sub: 0
                }
            )
            .to_vec(),
            [
                0x55, 0x53, 0x42, 0x43, 0x01, 0, 0, 0, 0x05, 0, 0, 0, 0x80, 0, 0x06, 0x01, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn flash_info_fields_read_little_endian() {
        let mut raw = vec![0u8; 512];
        raw[0..4].copy_from_slice(&1000u32.to_le_bytes());
        raw[4..6].copy_from_slice(&32u16.to_le_bytes());
        raw[6] = 4;
        let info = FlashInfo::from_reply(raw[..11].to_vec()).unwrap();
        assert_eq!(
            (info.sectors, info.block_sectors, info.page_sectors),
            (1000, 32, 4)
        );
        assert_eq!(info.bytes(), 512_000);
        assert!(FlashInfo::from_reply(vec![0; 10]).is_err());
    }
}
