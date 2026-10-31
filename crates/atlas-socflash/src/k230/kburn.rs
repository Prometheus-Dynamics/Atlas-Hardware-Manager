//! The K230 burner (stage 2, "kburn" in U-Boot): every command and reply is
//! a 60-byte packet, little-endian `cmd`, `result`, `data_size`, then 54
//! bytes of data. A reply's `cmd` is the request's with 0x8000 set; result
//! 1 is success, 2 failure, 0xFF failure with a message in the data.

use std::io::Read;

use sha2::{Digest, Sha256};

use crate::transport::{UsbError, UsbIo};
use crate::{Phase, Progress, hex};

pub const PACKET: usize = 60;
const DATA: usize = 54;
const REPLY_FLAG: u16 = 0x8000;
const STALE_REPLIES: usize = 8;

pub mod cmd {
    pub const NOP: u16 = 0x00;
    pub const REBOOT: u16 = 0x01;
    pub const PROBE: u16 = 0x10;
    pub const GET_INFO: u16 = 0x11;
    pub const ERASE: u16 = 0x20;
    pub const WRITE: u16 = 0x21;
    pub const VERIFY: u16 = 0x25;
}

const RESULT_OK: u16 = 1;
const RESULT_MESSAGE: u16 = 0xff;
/// "Rbot", the reboot command's guard.
const REBOOT_MARK: u64 = 0x5262_6f74;

/// The storage to burn (k230_flash's values; the C tool's header wasn't
/// read, so they're UNCONFIRMED beyond agreeing with the Python port).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Emmc = 1,
    Sd = 2,
    SpiNand = 3,
    SpiNor = 4,
}

impl Medium {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "emmc" => Some(Self::Emmc),
            "sd" => Some(Self::Sd),
            "spi-nand" => Some(Self::SpiNand),
            "spi-nor" => Some(Self::SpiNor),
            _ => None,
        }
    }
}

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

fn words(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The medium as the burner describes it (`DEV_GET_INFO`).
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
    /// Bytes per bulk write, from the probe.
    chunk: usize,
    max_packet: usize,
}

impl<'a> Burner<'a> {
    pub fn new(io: &'a mut dyn UsbIo) -> Self {
        let max_packet = io.max_packet_out();
        Self {
            io,
            chunk: 0,
            max_packet,
        }
    }

    /// Sends a command and returns its reply's data.
    async fn command(&mut self, cmd: u16, data: &[u8]) -> Result<Vec<u8>, UsbError> {
        self.io.bulk_out(&packet(cmd, data)).await?;
        self.reply(cmd).await
    }

    async fn reply(&mut self, cmd: u16) -> Result<Vec<u8>, UsbError> {
        for _ in 0..=STALE_REPLIES {
            let reply = self.io.bulk_in(PACKET).await?;
            if reply.len() < 6 {
                return Err(UsbError::Protocol(format!("a {}-byte reply", reply.len())));
            }
            if le16(&reply, 0) != cmd | REPLY_FLAG {
                continue; // a stale reply to an earlier command
            }
            let size = usize::from(le16(&reply, 4)).min(DATA).min(reply.len() - 6);
            let data = reply[6..6 + size].to_vec();
            return match le16(&reply, 2) {
                RESULT_OK => Ok(data),
                _ if cmd == cmd::NOP => Ok(data),
                RESULT_MESSAGE => Err(UsbError::Protocol(format!(
                    "the burner refused command {cmd:#04x}: {}",
                    String::from_utf8_lossy(&data).trim_matches('\0')
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

    /// Picks the medium; the burner says how much to send per bulk write.
    pub async fn probe(&mut self, medium: Medium) -> Result<(), UsbError> {
        let data = self.command(cmd::PROBE, &[medium as u8, 0xff]).await?;
        if data.len() < 16 {
            return Err(UsbError::Protocol(format!(
                "a {}-byte probe reply",
                data.len()
            )));
        }
        let chunk = le64(&data, 0);
        if chunk == 0 || chunk > 64 << 20 {
            return Err(UsbError::Protocol(format!(
                "a write chunk of {chunk} bytes"
            )));
        }
        self.chunk = chunk as usize;
        Ok(())
    }

    pub async fn info(&mut self) -> Result<MediumInfo, UsbError> {
        let data = self.command(cmd::GET_INFO, &[]).await?;
        if data.len() < 32 {
            return Err(UsbError::Protocol(format!(
                "a {}-byte info reply",
                data.len()
            )));
        }
        let bits = le64(&data, 24);
        Ok(MediumInfo {
            capacity: le64(&data, 0),
            block_size: le64(&data, 8),
            erase_size: le64(&data, 16),
            write_protected: (bits >> 32) & 0xff != 0,
            valid: (bits >> 47) & 1 == 1,
        })
    }

    pub async fn erase(&mut self, offset: u64, size: u64) -> Result<(), UsbError> {
        self.command(cmd::ERASE, &words(&[offset, size]))
            .await
            .map(drop)
    }

    /// Writes `size` bytes of `image` at `offset`, into a region of
    /// `region` bytes (padded by the burner; on NAND it skips bad blocks
    /// inside it). Returns the SHA-256 of what was sent.
    pub async fn write(
        &mut self,
        offset: u64,
        image: &mut (dyn Read + Send),
        size: u64,
        region: u64,
        progress: Progress<'_>,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<String, UsbError> {
        if self.chunk == 0 {
            return Err(UsbError::Protocol("write before probe".into()));
        }
        self.command(cmd::WRITE, &words(&[offset, size, region, 0]))
            .await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; self.chunk];
        let mut sent = 0u64;
        while sent < size {
            if cancelled() {
                return Err(UsbError::Other(
                    "cancelled mid-write; the medium is incomplete".into(),
                ));
            }
            let take = (size - sent).min(self.chunk as u64) as usize;
            image
                .read_exact(&mut buffer[..take])
                .map_err(|error| UsbError::Other(format!("reading the image: {error}")))?;
            hasher.update(&buffer[..take]);
            self.io.bulk_out(&buffer[..take]).await?;
            // A short write that fills its last packet exactly needs a
            // zero-length packet to end the transfer.
            if take < self.chunk && take.is_multiple_of(self.max_packet) {
                self.io.bulk_out(&[]).await?;
            }
            sent += take as u64;
            progress(Phase::Writing, sent, size);
        }
        self.reply(cmd::WRITE).await?;
        self.command(cmd::NOP, &[]).await?;
        Ok(hex(&hasher.finalize()))
    }

    /// The burner's SHA-256 of `size` bytes at `offset` (burner version 2;
    /// the reply's layout beyond the first 32 bytes is UNCONFIRMED).
    pub async fn sha256(&mut self, offset: u64, size: u64) -> Result<String, UsbError> {
        let data = self.command(cmd::VERIFY, &words(&[offset, size])).await?;
        if data.len() < 32 {
            return Err(UsbError::Protocol(format!(
                "a {}-byte verify reply",
                data.len()
            )));
        }
        Ok(hex(&data[..32]))
    }

    pub async fn reboot(&mut self) -> Result<(), UsbError> {
        match self
            .io
            .bulk_out(&packet(cmd::REBOOT, &REBOOT_MARK.to_le_bytes()))
            .await
        {
            Err(UsbError::Disconnected) => Ok(()),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Script, Step};

    fn hex_bytes(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    fn reply(cmd: u16, result: u16, data: &[u8]) -> Step {
        let mut packet = packet(cmd | REPLY_FLAG, data);
        packet[2..4].copy_from_slice(&result.to_le_bytes());
        Step::BulkIn {
            max: PACKET,
            reply: packet.to_vec(),
        }
    }

    #[test]
    fn packets_match_the_spec_vectors() {
        let mut probe = hex_bytes("10000000020003ff");
        probe.resize(60, 0);
        assert_eq!(packet(cmd::PROBE, &[3, 0xff]).to_vec(), probe);
        let write = packet(cmd::WRITE, &words(&[0, 0x1000, 0x1000, 0]));
        assert_eq!(&write[..6], &hex_bytes("210000002000"));
        assert_eq!(&write[14..16], &[0x00, 0x10]);
    }

    #[tokio::test]
    async fn probe_info_and_a_write_with_its_zero_length_packet() {
        let image: Vec<u8> = (0..1536u32).map(|i| i as u8).collect();
        let mut script = Script::new(vec![
            Step::BulkOut(packet(cmd::PROBE, &[3, 0xff]).to_vec()),
            reply(cmd::PROBE, 1, &words(&[1024, 512])),
            Step::BulkOut(packet(cmd::GET_INFO, &[]).to_vec()),
            reply(
                cmd::GET_INFO,
                1,
                &words(&[128 << 20, 2048, 128 << 10, (1 << 47) | 3000]),
            ),
            Step::BulkOut(packet(cmd::WRITE, &words(&[0x10_0000, 1536, 0x20_0000, 0])).to_vec()),
            // A stale reply first, which is skipped.
            reply(cmd::GET_INFO, 1, &[]),
            reply(cmd::WRITE, 1, &[0; 8]),
            Step::BulkOut(image[..1024].to_vec()),
            Step::BulkOut(image[1024..].to_vec()),
            Step::BulkOut(Vec::new()),
            reply(cmd::WRITE, 1, &[]),
            Step::BulkOut(packet(cmd::NOP, &[]).to_vec()),
            reply(cmd::NOP, 0, &[0; 16]),
        ]);
        let mut burner = Burner::new(&mut script);
        burner.probe(Medium::SpiNand).await.unwrap();
        let info = burner.info().await.unwrap();
        assert_eq!(
            (
                info.capacity,
                info.erase_size,
                info.valid,
                info.write_protected
            ),
            (128 << 20, 128 << 10, true, false)
        );
        let sha = burner
            .write(
                0x10_0000,
                &mut &image[..],
                1536,
                0x20_0000,
                &mut |_, _, _| {},
                &|| false,
            )
            .await
            .unwrap();
        assert_eq!(sha, hex(&Sha256::digest(&image)));
        assert!(script.finished());
    }

    #[tokio::test]
    async fn the_burners_message_comes_through() {
        let mut script = Script::new(vec![
            Step::BulkOut(packet(cmd::ERASE, &words(&[0, 4096])).to_vec()),
            reply(cmd::ERASE, 0xff, b"no medium"),
        ]);
        let mut burner = Burner::new(&mut script);
        let error = burner.erase(0, 4096).await.unwrap_err();
        assert!(error.to_string().contains("no medium"), "{error}");
    }
}
