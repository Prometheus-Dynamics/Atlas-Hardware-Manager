//! The K230 burner (stage 2, "kburn" in U-Boot): every command and reply is
//! a 60-byte packet, little-endian `cmd`, `result`, `data_size`, then 54
//! bytes of data. A reply's `cmd` is the request's with 0x8000 set; result
//! 1 is success, 2 failure, 0xFF failure with a message in the data.
//!
//! Follows k230_flash (33a75f0) burner_uboot.cpp; "C:n" below is a line of
//! that file. Tests: tests/k230_vectors.rs.

use std::io::Read;

use sha2::{Digest, Sha256};

use super::{show, trace};
use crate::transport::{UsbError, UsbIo};
use crate::{Phase, Progress, hex};

/// burner_uboot.cpp:46 `KBUNR_USB_PKT_SIZE`.
pub const PACKET: usize = 60;
/// C:50-59: a 6-byte header, then this much data.
pub const DATA: usize = 54;
const REPLY_FLAG: u16 = 0x8000; // C:13
const STALE_REPLIES: usize = 8; // C:14

/// C:16-33 `kburn_pkt_cmd`.
pub mod cmd {
    pub const NOP: u16 = 0x00;
    pub const REBOOT: u16 = 0x01;
    pub const PROBE: u16 = 0x10;
    pub const GET_INFO: u16 = 0x11;
    pub const ERASE: u16 = 0x20;
    pub const WRITE: u16 = 0x21;
    pub const WRITE_CHUNK: u16 = 0x22;
    pub const READ: u16 = 0x23;
    pub const READ_CHUNK: u16 = 0x24;
    pub const VERIFY: u16 = 0x25;
}

/// C:35-41 `kburn_pkt_result`.
pub const RESULT_OK: u16 = 1;
pub const RESULT_ERROR: u16 = 2;
pub const RESULT_MESSAGE: u16 = 0xff;
/// "Rbot", the reboot command's guard (C:`REBOOT_MARK`, sent as a u64).
pub const REBOOT_MARK: u64 = 0x5262_6f74;

/// The storage to burn: kburn.h:39-44 `KBurnMediumType` (INVALID 0, EMMC 1,
/// SDCARD 2, SPI_NAND 3, SPI_NOR 4, OTP 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Emmc = 1,
    Sd = 2,
    SpiNand = 3,
    SpiNor = 4,
    Otp = 5,
}

impl Medium {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "emmc" => Some(Self::Emmc),
            "sd" => Some(Self::Sd),
            "spi-nand" => Some(Self::SpiNand),
            "spi-nor" => Some(Self::SpiNor),
            "otp" => Some(Self::Otp),
            _ => None,
        }
    }
}

/// A command packet: `data` goes after the header, the rest is zero.
pub fn packet(cmd: u16, data: &[u8]) -> [u8; PACKET] {
    let mut packet = [0u8; PACKET];
    packet[0..2].copy_from_slice(&cmd.to_le_bytes());
    packet[4..6].copy_from_slice(&(data.len() as u16).to_le_bytes());
    packet[6..6 + data.len()].copy_from_slice(data);
    packet
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

/// Little-endian u64 words, the layout of every command's data.
pub fn words(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn refuse(message: impl Into<String>) -> UsbError {
    UsbError::Other(message.into())
}

/// The medium as the burner describes it (`DEV_GET_INFO`: C:`kburn_medium_info`,
/// kburn_k230.h:46-56).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediumInfo {
    pub capacity: u64,
    pub block_size: u64,
    pub erase_size: u64,
    pub write_protected: bool,
    pub valid: bool,
}

pub struct Burner<'a> {
    io: &'a mut dyn UsbIo,
    /// Bytes per bulk write and per bulk read, from the probe.
    out_chunk: u64,
    in_chunk: u64,
    max_packet: usize,
    /// 0 until read; C:kburn_probe_loader_version.
    loader_version: u32,
    synced: bool,
    info: Option<MediumInfo>,
    /// The last request, hex, for the transfer log.
    sent: String,
}

impl<'a> Burner<'a> {
    pub fn new(io: &'a mut dyn UsbIo) -> Self {
        let max_packet = io.max_packet_out();
        Self {
            io,
            out_chunk: 0,
            in_chunk: 0,
            max_packet,
            loader_version: 0,
            synced: false,
            info: None,
            sent: String::new(),
        }
    }

    async fn send(&mut self, cmd: u16, data: &[u8]) -> Result<(), UsbError> {
        let packet = packet(cmd, data);
        self.sent = show(&packet);
        self.io.bulk_out(&packet).await
    }

    /// Sends a command and returns its reply's data.
    async fn command(
        &mut self,
        cmd: u16,
        data: &[u8],
        expected: &str,
    ) -> Result<Vec<u8>, UsbError> {
        self.send(cmd, data).await?;
        self.reply(cmd, expected).await
    }

    /// C:`kburn_read_response` (C:295-329) and `kburn_parse_resp` (C:241-293): replies
    /// are whole 60-byte packets; one whose command isn't the awaited one,
    /// whose size is over 54, or (for NOP) whose result isn't OK is a stale
    /// reply and is skipped, at most 8 times.
    async fn reply(&mut self, cmd: u16, expected: &str) -> Result<Vec<u8>, UsbError> {
        let awaited = cmd | REPLY_FLAG;
        for _ in 0..=STALE_REPLIES {
            let reply = self.io.bulk_in(PACKET).await?;
            let step = format!("kburn command {cmd:#04x}");
            trace(
                &step,
                &self.sent,
                &reply,
                &format!("cmd {awaited:#06x} result 1 {expected}"),
            );
            if reply.len() != PACKET {
                return Err(UsbError::Protocol(format!(
                    "a {}-byte reply, not {PACKET}",
                    reply.len()
                )));
            }
            let (got, result, size) = (
                le16(&reply, 0),
                le16(&reply, 2),
                usize::from(le16(&reply, 4)),
            );
            if size > DATA || got != awaited || (cmd == cmd::NOP && result != RESULT_OK) {
                continue; // a stale reply to an earlier command
            }
            let data = reply[6..6 + size].to_vec();
            return match result {
                RESULT_OK => Ok(data),
                RESULT_MESSAGE => Err(UsbError::Protocol(format!(
                    "the burner refused command {cmd:#04x} (result {result:#04x}): {}",
                    String::from_utf8_lossy(&data)
                        .split('\0')
                        .next()
                        .unwrap_or_default()
                ))),
                result => Err(UsbError::Protocol(format!(
                    "the burner failed command {cmd:#04x} (result {result})"
                ))),
            };
        }
        Err(UsbError::Protocol(format!(
            "no reply to command {cmd:#04x} after {STALE_REPLIES} stale ones"
        )))
    }

    /// C:872-897 (the constructor): the loader version (vendor IN request 0,
    /// wValue 1, 4 bytes; any failure reads as 0), then a NOP to clear the
    /// burner's error state.
    async fn sync(&mut self) -> Result<(), UsbError> {
        if self.synced {
            return Ok(());
        }
        self.loader_version = match self.io.control_in(0, 1, 0, 4).await {
            Ok(bytes) if bytes.len() == 4 => {
                let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                trace(
                    "kburn loader version",
                    "IN vendor req 0 wValue 1 wIndex 0 len 4",
                    &bytes,
                    "4 bytes, a little-endian version (0, 1 or 2)",
                );
                version
            }
            Err(UsbError::Disconnected) => return Err(UsbError::Disconnected),
            other => {
                trace(
                    "kburn loader version",
                    "IN vendor req 0 wValue 1 wIndex 0 len 4",
                    other.as_deref().unwrap_or_default(),
                    "4 bytes; anything else counts as version 0",
                );
                0
            }
        };
        self.command(cmd::NOP, &[], "size 0").await?;
        self.synced = true;
        Ok(())
    }

    /// The loader's protocol version (0 before the first probe).
    pub fn loader_version(&self) -> u32 {
        self.loader_version
    }

    /// Picks the medium (C:`kburn_probe`, C:503-543); the burner says how
    /// much to send per bulk write and read.
    pub async fn probe(&mut self, medium: Medium) -> Result<(), UsbError> {
        self.sync().await?;
        let data = self
            .command(
                cmd::PROBE,
                &[medium as u8, 0xff],
                "size 16: out chunk u64 (1..=INT_MAX), in chunk u64 (1..=65535)",
            )
            .await?;
        if data.len() != 16 {
            return Err(UsbError::Protocol(format!(
                "a {}-byte probe reply, not 16",
                data.len()
            )));
        }
        let (out, input) = (le64(&data, 0), le64(&data, 8));
        if out == 0 || out > i32::MAX as u64 || input == 0 || input > u64::from(u16::MAX) {
            return Err(UsbError::Protocol(format!(
                "chunk sizes {out} out and {input} in"
            )));
        }
        self.out_chunk = out;
        self.in_chunk = input;
        Ok(())
    }

    /// C:`kburn_get_capacity` (C:545-568): the reply is exactly the 32-byte struct.
    pub async fn info(&mut self) -> Result<MediumInfo, UsbError> {
        let data = self
            .command(
                cmd::GET_INFO,
                &[],
                "size 32: capacity, block, erase size, then timeout_ms:32 wp:8 type:7 valid:1",
            )
            .await?;
        if data.len() != 32 {
            return Err(UsbError::Protocol(format!(
                "a {}-byte info reply, not 32",
                data.len()
            )));
        }
        let bits = le64(&data, 24);
        let info = MediumInfo {
            capacity: le64(&data, 0),
            block_size: le64(&data, 8),
            erase_size: le64(&data, 16),
            write_protected: (bits >> 32) & 0xff != 0,
            valid: (bits >> 47) & 1 == 1,
        };
        self.info = Some(info);
        Ok(info)
    }

    /// C:`kburn_erase` (C:570-631): bounds and write protection are checked first (the
    /// C tool checks `wp == 1`; any set byte refuses here), and a reply
    /// that times out is waited for again, 3 to 120 times.
    pub async fn erase(&mut self, offset: u64, size: u64) -> Result<(), UsbError> {
        let mut retries = 3;
        if let Some(info) = self.info {
            if offset > info.capacity || size > info.capacity - offset {
                return Err(refuse("the erase runs past the medium"));
            }
            if info.write_protected {
                return Err(refuse("the medium is write-protected"));
            }
            retries = (size / info.erase_size.max(1) + 2).clamp(3, 120);
        }
        self.send(cmd::ERASE, &words(&[offset, size])).await?;
        let mut tried = 0;
        loop {
            match self.reply(cmd::ERASE, "size 0").await {
                Err(UsbError::Timeout(_)) if tried < retries => tried += 1,
                other => return other.map(drop),
            }
        }
    }

    /// Writes `size` bytes of `image` at `offset`, into a region of
    /// `region` bytes (padded by the burner; on NAND it skips bad blocks
    /// inside it). As the C tool does, the size sent is rounded up to the
    /// block size and the tail is zero padding; `info` must have run.
    /// Returns the SHA-256 of the `size` image bytes.
    pub async fn write(
        &mut self,
        offset: u64,
        image: &mut (dyn Read + Send),
        size: u64,
        region: u64,
        progress: Progress<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<String, UsbError> {
        let (Some(info), true) = (self.info, self.out_chunk != 0) else {
            return Err(refuse("write before probe and info"));
        };
        let block = info.block_size;
        if size == 0 || block == 0 || info.erase_size == 0 {
            return Err(refuse("nothing to write, or a medium without blocks"));
        }
        let total = size.div_ceil(block) * block;
        let chunk = self.out_chunk / block * block;
        if chunk == 0 {
            return Err(refuse("the write chunk is smaller than a block"));
        }
        // C:`kburn_write_start` (C:633-709).
        if offset > info.capacity || total > info.capacity - offset {
            return Err(refuse("the write runs past the medium"));
        }
        if region != 0 && (total > region || region > info.capacity - offset) {
            return Err(refuse("the write exceeds its region"));
        }
        if info.write_protected {
            return Err(refuse("the medium is write-protected"));
        }
        if !offset.is_multiple_of(info.erase_size) {
            return Err(refuse("the write start isn't aligned to the erase size"));
        }
        // Protocol 1 added the partition flag word.
        let cfg = if self.loader_version >= 1 {
            words(&[offset, total, region, 0])
        } else {
            words(&[offset, total, region])
        };
        self.command(cmd::WRITE, &cfg, "size 0 (the start)").await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; chunk as usize];
        let mut sent = 0u64;
        while sent < total {
            if cancelled() {
                return Err(UsbError::Other(
                    "cancelled mid-write; the medium is incomplete".into(),
                ));
            }
            let take = (total - sent).min(chunk) as usize;
            let logical = size.saturating_sub(sent).min(take as u64) as usize;
            image
                .read_exact(&mut buffer[..logical])
                .map_err(|error| UsbError::Other(format!("reading the image: {error}")))?;
            hasher.update(&buffer[..logical]);
            buffer[logical..take].fill(0);
            self.write_chunk(&buffer[..take], self.out_chunk.min(total - sent))
                .await?;
            sent += take as u64;
            progress(Phase::Writing, sent.min(size), size);
        }
        self.reply(cmd::WRITE, "size 0 (the end)").await?;
        self.command(cmd::NOP, &[], "size 0").await?;
        Ok(hex(&hasher.finalize()))
    }

    /// C:`kburn_write_chunk` (C:711-751): a chunk shorter than the burner
    /// expects that fills its last packet exactly is followed by a
    /// zero-length packet; a failed write reads the burner's reply for the
    /// reason.
    async fn write_chunk(&mut self, data: &[u8], expected: u64) -> Result<(), UsbError> {
        if let Err(error) = self.io.bulk_out(data).await {
            if error == UsbError::Disconnected {
                return Err(error);
            }
            return match self.reply(cmd::WRITE, "an error reply").await {
                Err(reason) => Err(reason),
                Ok(_) => Err(error),
            };
        }
        if (data.len() as u64) < expected && data.len().is_multiple_of(self.max_packet) {
            self.io.bulk_out(&[]).await?;
        }
        Ok(())
    }

    /// C:`K230UBOOTBurner::read` (C:1076-1150): `size` bytes at `offset`,
    /// read as whole blocks in chunks of the probe's input size.
    pub async fn read(&mut self, offset: u64, size: u64) -> Result<Vec<u8>, UsbError> {
        let (Some(info), true) = (self.info, self.in_chunk != 0) else {
            return Err(refuse("read before probe and info"));
        };
        let block = info.block_size;
        if size == 0 || block == 0 || offset > info.capacity {
            return Err(refuse("nothing to read, or a medium without blocks"));
        }
        let start = offset - offset % block;
        let prefix = offset - start;
        let span = (prefix + size).div_ceil(block) * block;
        if span > info.capacity - start {
            return Err(refuse("the read runs past the medium"));
        }
        self.command(cmd::READ, &words(&[start, span]), "size 0 (the start)")
            .await?;
        let mut data = Vec::with_capacity(span as usize);
        while (data.len() as u64) < span {
            let take = (span - data.len() as u64).min(self.in_chunk) as usize;
            data.extend_from_slice(&self.read_chunk(take).await?);
        }
        self.reply(cmd::READ_CHUNK, "size 0 (the end)").await?;
        Ok(data[prefix as usize..(prefix + size) as usize].to_vec())
    }

    /// C:`kburn_read_chunk` (C:804-857): one whole packet of 6 + `size` bytes, retried
    /// on a timeout up to 3 times.
    async fn read_chunk(&mut self, size: usize) -> Result<Vec<u8>, UsbError> {
        let mut tried = 0;
        let reply = loop {
            match self.io.bulk_in(6 + size).await {
                Err(UsbError::Timeout(_)) if tried < 3 => tried += 1,
                other => break other?,
            }
        };
        trace(
            "kburn read chunk",
            &self.sent,
            &reply,
            &format!("cmd 0x8024 result 1 size {size}, then {size} data bytes"),
        );
        if reply.len() != 6 + size
            || le16(&reply, 0) != cmd::READ_CHUNK | REPLY_FLAG
            || le16(&reply, 2) != RESULT_OK
            || usize::from(le16(&reply, 4)) != size
        {
            return Err(UsbError::Protocol(format!(
                "a bad read chunk: {} bytes, cmd {:#06x}, result {}",
                reply.len(),
                reply.first().map_or(0, |_| le16(&reply, 0)),
                reply.get(3).map_or(0, |_| le16(&reply, 2)),
            )));
        }
        Ok(reply[6..].to_vec())
    }

    /// The burner's SHA-256 of `size` bytes at `offset` (C:`kburn_verify_sha256`,
    /// C:369-432; loader version 2 and up). The reply is exactly 48 bytes:
    /// the digest, the byte count it covered, the milliseconds it took; any
    /// other length or a different count is an error, not a match. (The C
    /// tool also raises the USB timeout to 300 s; the transport's timeout
    /// is fixed at open, UNCONFIRMED whether that is long enough.)
    pub async fn sha256(&mut self, offset: u64, size: u64) -> Result<String, UsbError> {
        if self.loader_version < 2 {
            return Err(refuse("the loader does not support verification"));
        }
        let data = self
            .command(
                cmd::VERIFY,
                &words(&[offset, size]),
                "size 48: sha256, verified bytes u64, elapsed ms u64",
            )
            .await?;
        if data.len() != 48 {
            return Err(UsbError::Protocol(format!(
                "a {}-byte verify reply, not 48",
                data.len()
            )));
        }
        let verified = le64(&data, 32);
        if verified != size {
            return Err(UsbError::Protocol(format!(
                "the burner verified {verified} bytes, not {size}"
            )));
        }
        Ok(hex(&data[..32]))
    }

    /// C:`kburn_reset_chip` (C:484-501): the command with the mark as a u64, no
    /// reply read. A disconnect after it is the reboot.
    pub async fn reboot(&mut self) -> Result<(), UsbError> {
        match self.send(cmd::REBOOT, &REBOOT_MARK.to_le_bytes()).await {
            Err(UsbError::Disconnected) => Ok(()),
            other => other,
        }
    }
}
