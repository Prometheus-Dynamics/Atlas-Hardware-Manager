//! The FEL wire format: the 32-byte host request block, the 13-byte status
//! block the device ends each transfer with, and the 16-byte command
//! messages. Field order, sizes and command numbers come from the field
//! tables of the linux-sunxi FEL/Protocol page (see fel/mod.rs for the
//! archive URL). The encoders and decoders here are written from those
//! tables.

use atlas_usb::UsbError;

/// Host request block: "AWUC", tag, length, then the command byte.
pub const REQUEST_LEN: usize = 32;
/// Device status block: "AWUS", tag, residue, status byte.
pub const CSW_LEN: usize = 13;
/// Status response: mark 0xffff, tag, state byte, three reserved bytes.
pub const STATUS_LEN: usize = 8;
/// Standard command (no address): cmd, tag, twelve reserved bytes.
pub const STANDARD_LEN: usize = 16;
/// Message command (address and length): cmd, tag, address, length, flags.
pub const MESSAGE_LEN: usize = 16;
/// Verify-device response: "AWUSBFEX", board, firmware, mode, flags, scratchpad.
pub const VERIFY_LEN: usize = 32;

const REQUEST_MAGIC: &[u8; 4] = b"AWUC";
const STATUS_MAGIC: &[u8; 4] = b"AWUS";
const VERIFY_MAGIC: &[u8; 8] = b"AWUSBFEX";

/// The host-side direction byte in the request block.
pub const AW_USB_READ: u8 = 0x11;
pub const AW_USB_WRITE: u8 = 0x12;

/// Command numbers (the FEL/Protocol page's request list).
pub mod cmd {
    pub const VERIFY_DEVICE: u16 = 0x0001;
    pub const SWITCH_ROLE: u16 = 0x0002;
    pub const IS_READY: u16 = 0x0003;
    pub const GET_CMD_SET_VER: u16 = 0x0004;
    pub const DISCONNECT: u16 = 0x0010;
    pub const DOWNLOAD: u16 = 0x0101;
    pub const RUN: u16 = 0x0102;
    pub const UPLOAD: u16 = 0x0103;
}

/// The verify-device mode the FEL state reports (the page's
/// AL_VERIFY_DEV_MODE_FEL).
pub const MODE_FEL: u16 = 1;

/// The largest data stage the page gives for DOWNLOAD (and the read stage
/// Atlas uses for UPLOAD, the same bound).
pub const MAX_TRANSFER: usize = 65_536;

/// A 32-byte host request block for `direction` with `len` bytes after it.
pub fn request(direction: u8, len: u32) -> [u8; REQUEST_LEN] {
    let mut block = [0u8; REQUEST_LEN];
    block[0..4].copy_from_slice(REQUEST_MAGIC);
    // Bytes 4..8 are the tag, zero.
    block[8..12].copy_from_slice(&len.to_le_bytes());
    // Bytes 12..14 are reserved, 14 is zero; 15 is the command length, 0xc.
    block[15] = 0x0c;
    block[16] = direction;
    // Byte 17 is reserved; the second length repeats the first.
    block[18..22].copy_from_slice(&len.to_le_bytes());
    block
}

/// A 16-byte standard command (no address), for example VERIFY_DEVICE.
pub fn standard(command: u16) -> [u8; STANDARD_LEN] {
    let mut body = [0u8; STANDARD_LEN];
    body[0..2].copy_from_slice(&command.to_le_bytes());
    body
}

/// A 16-byte message command: `command` on `address` for `len` bytes.
pub fn message(command: u16, address: u32, len: u32) -> [u8; MESSAGE_LEN] {
    let mut body = [0u8; MESSAGE_LEN];
    body[0..2].copy_from_slice(&command.to_le_bytes());
    body[4..8].copy_from_slice(&address.to_le_bytes());
    body[8..12].copy_from_slice(&len.to_le_bytes());
    body
}

/// The device's 13-byte status block at the end of a transfer. A status
/// byte other than zero means the command failed.
pub fn parse_csw(bytes: &[u8]) -> Result<u8, UsbError> {
    if bytes.len() != CSW_LEN || &bytes[0..4] != STATUS_MAGIC {
        return Err(UsbError::Protocol(format!(
            "expected a {CSW_LEN}-byte status block, got {} bytes",
            bytes.len()
        )));
    }
    Ok(bytes[12])
}

/// The verify-device response (FEL_VERIFY_DEVICE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verify {
    /// The SoC id the device reports (the page's "board" field).
    pub board: u32,
    pub firmware: u32,
    pub mode: u16,
    pub data_flag: u8,
    pub data_length: u8,
    /// Where the device's scratch data starts (the page's
    /// data_start_address); FEL_UPLOAD can read there.
    pub scratchpad: u32,
}

pub fn parse_verify(bytes: &[u8]) -> Result<Verify, UsbError> {
    if bytes.len() != VERIFY_LEN || &bytes[0..8] != VERIFY_MAGIC {
        return Err(UsbError::Protocol(format!(
            "the verify-device response is not AWUSBFEX ({} bytes)",
            bytes.len()
        )));
    }
    let u32_at =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Ok(Verify {
        board: u32_at(8),
        firmware: u32_at(12),
        mode: u16::from_le_bytes([bytes[16], bytes[17]]),
        data_flag: bytes[18],
        data_length: bytes[19],
        scratchpad: u32_at(20),
    })
}

/// The 8-byte status response after a transfer. `state` zero is success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub mark: u16,
    pub tag: u16,
    pub state: u8,
}

pub fn parse_status(bytes: &[u8]) -> Result<Status, UsbError> {
    if bytes.len() != STATUS_LEN {
        return Err(UsbError::Protocol(format!(
            "the status response is {} bytes, not {STATUS_LEN}",
            bytes.len()
        )));
    }
    Ok(Status {
        mark: u16::from_le_bytes([bytes[0], bytes[1]]),
        tag: u16::from_le_bytes([bytes[2], bytes[3]]),
        state: bytes[4],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_block_has_the_page_fields() {
        let block = request(AW_USB_WRITE, 16);
        assert_eq!(block.len(), REQUEST_LEN);
        assert_eq!(&block[0..4], b"AWUC");
        assert_eq!(&block[8..12], &16u32.to_le_bytes());
        assert_eq!(block[15], 0x0c);
        assert_eq!(block[16], AW_USB_WRITE);
        assert_eq!(&block[18..22], &16u32.to_le_bytes());
    }

    #[test]
    fn the_messages_carry_command_address_and_length() {
        let body = message(cmd::DOWNLOAD, 0x0004_0000, 512);
        assert_eq!(&body[0..2], &0x0101u16.to_le_bytes());
        assert_eq!(&body[4..8], &0x0004_0000u32.to_le_bytes());
        assert_eq!(&body[8..12], &512u32.to_le_bytes());
        assert_eq!(standard(cmd::VERIFY_DEVICE)[0], 1);
    }

    #[test]
    fn the_status_block_must_be_thirteen_bytes_and_say_awus() {
        let mut csw = b"AWUS".to_vec();
        csw.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(parse_csw(&csw).unwrap(), 1);
        assert!(parse_csw(&csw[..12]).is_err());
        assert!(parse_csw(b"AWUC000000000").is_err());
    }

    #[test]
    fn the_verify_response_reads_its_fields() {
        let mut bytes = vec![0u8; VERIFY_LEN];
        bytes[0..8].copy_from_slice(b"AWUSBFEX");
        bytes[8..12].copy_from_slice(&0x0016_1000u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        bytes[16..18].copy_from_slice(&MODE_FEL.to_le_bytes());
        bytes[18] = 0x44;
        bytes[19] = 0x08;
        bytes[20..24].copy_from_slice(&0x7e00u32.to_le_bytes());
        let verify = parse_verify(&bytes).unwrap();
        assert_eq!(verify.board, 0x0016_1000);
        assert_eq!(verify.mode, MODE_FEL);
        assert_eq!(verify.scratchpad, 0x7e00);
        bytes[0] = b'X';
        assert!(parse_verify(&bytes).is_err());
    }

    #[test]
    fn the_status_response_is_eight_bytes() {
        let status = parse_status(&[0xff, 0xff, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(status.mark, 0xffff);
        assert_eq!(status.state, 0);
        assert!(parse_status(&[0; 7]).is_err());
    }
}
