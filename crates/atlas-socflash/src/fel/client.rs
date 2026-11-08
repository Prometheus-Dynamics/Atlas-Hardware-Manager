//! The FEL client: each FEL command as the transfers the protocol makes
//! over a [`UsbIo`]. A command is a request block, the command body, then
//! the device's status block. A data stage is a request block for its
//! direction, the bytes, then the status block; a read's bytes come before
//! its status block. Every command's status byte must be zero. Wire format
//! and commands: fel/protocol.rs.

use super::protocol::{
    AW_USB_READ, AW_USB_WRITE, CSW_LEN, MAX_TRANSFER, STATUS_LEN, Status, Verify, cmd, message,
    parse_csw, parse_status, parse_verify, request, standard,
};
use super::trace;
use crate::transport::{UsbError, UsbIo};

/// One FEL session over a download-mode device.
pub struct Fel<'a> {
    io: &'a mut dyn UsbIo,
}

impl<'a> Fel<'a> {
    pub fn new(io: &'a mut dyn UsbIo) -> Self {
        Self { io }
    }

    /// The device's status block: zero is success.
    async fn status_block(&mut self, what: &str) -> Result<(), UsbError> {
        let block = self.io.bulk_in(CSW_LEN).await?;
        let status = parse_csw(&block)?;
        trace(what, "status block", &block, "status byte 0");
        if status != 0 {
            return Err(UsbError::Protocol(format!(
                "{what} failed: the device's status byte is {status}"
            )));
        }
        Ok(())
    }

    /// A command body (a standard or message command) and its status.
    async fn command(&mut self, what: &str, body: &[u8]) -> Result<(), UsbError> {
        self.io
            .bulk_out(&request(AW_USB_WRITE, body.len() as u32))
            .await?;
        self.io.bulk_out(body).await?;
        self.status_block(what).await
    }

    /// A data stage the host sends: request, bytes, status. A chunk that
    /// fills its last packet ends with a zero-length packet, as the
    /// transport asks.
    async fn send_data(&mut self, what: &str, data: &[u8]) -> Result<(), UsbError> {
        self.io
            .bulk_out(&request(AW_USB_WRITE, data.len() as u32))
            .await?;
        self.io.bulk_out(data).await?;
        let packet = self.io.max_packet_out();
        if !data.is_empty() && data.len().is_multiple_of(packet) {
            self.io.bulk_out(&[]).await?;
        }
        self.status_block(what).await
    }

    /// A data stage the host reads: request, bytes (in as many transfers as
    /// the device needs), status.
    async fn read_data(&mut self, what: &str, len: usize) -> Result<Vec<u8>, UsbError> {
        self.io.bulk_out(&request(AW_USB_READ, len as u32)).await?;
        let mut data = Vec::with_capacity(len);
        while data.len() < len {
            let chunk = self.io.bulk_in(len - data.len()).await?;
            if chunk.is_empty() {
                return Err(UsbError::Protocol(format!(
                    "{what}: the device sent no bytes"
                )));
            }
            data.extend_from_slice(&chunk);
        }
        self.status_block(what).await?;
        Ok(data)
    }

    /// FEL_VERIFY_DEVICE: the SoC id, the mode and the scratch address.
    pub async fn verify(&mut self) -> Result<Verify, UsbError> {
        self.command("verify device", &standard(cmd::VERIFY_DEVICE))
            .await?;
        let bytes = self
            .read_data("verify device", super::protocol::VERIFY_LEN)
            .await?;
        let verify = parse_verify(&bytes)?;
        trace(
            "verify device",
            "FEL_VERIFY_DEVICE then a 32-byte read",
            &bytes,
            "AWUSBFEX with mode 1 (FEL)",
        );
        Ok(verify)
    }

    /// FEL_IS_READY: the status block after a transfer. The page reads 8
    /// bytes; state zero means the last command finished.
    pub async fn status(&mut self) -> Result<Status, UsbError> {
        let bytes = self.read_data("status", STATUS_LEN).await?;
        let status = parse_status(&bytes)?;
        if status.state != 0 {
            return Err(UsbError::Protocol(format!(
                "the device's state is {} after a transfer",
                status.state
            )));
        }
        Ok(status)
    }

    /// FEL_UPLOAD: `len` bytes from `address`, in 64 KiB stages.
    pub async fn read(&mut self, address: u32, len: usize) -> Result<Vec<u8>, UsbError> {
        let mut out = Vec::with_capacity(len);
        let mut at = address;
        while out.len() < len {
            let n = (len - out.len()).min(MAX_TRANSFER);
            self.command("upload", &message(cmd::UPLOAD, at, n as u32))
                .await?;
            let chunk = self.read_data("upload", n).await?;
            out.extend_from_slice(&chunk);
            at = at.wrapping_add(n as u32);
        }
        Ok(out)
    }

    /// FEL_DOWNLOAD: `data` to `address`, one message and one data stage
    /// per 64 KiB, each followed by its status read.
    pub async fn write(&mut self, address: u32, data: &[u8]) -> Result<(), UsbError> {
        let mut at = address;
        for chunk in data.chunks(MAX_TRANSFER) {
            self.command("download", &message(cmd::DOWNLOAD, at, chunk.len() as u32))
                .await?;
            self.send_data("download", chunk).await?;
            self.status().await?;
            at = at.wrapping_add(chunk.len() as u32);
        }
        Ok(())
    }

    /// FEL_RUN: jumps to `address`. The next command is served once the
    /// code returns (the FEL loop resumes), or the device has gone.
    pub async fn run(&mut self, address: u32) -> Result<(), UsbError> {
        self.command("run", &message(cmd::RUN, address, 0)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Script, Step};

    #[tokio::test]
    async fn verify_is_a_standard_command_then_a_32_byte_read() {
        let mut verify_reply = vec![0u8; 32];
        verify_reply[0..8].copy_from_slice(b"AWUSBFEX");
        verify_reply[8..12].copy_from_slice(&0x0016_1000u32.to_le_bytes());
        verify_reply[16..18].copy_from_slice(&1u16.to_le_bytes());
        let csw = |status: u8| {
            let mut csw = b"AWUS".to_vec();
            csw.extend_from_slice(&[0; 8]);
            csw.push(status);
            csw
        };
        let mut script = Script::new(vec![
            Step::BulkOut(request(AW_USB_WRITE, 16).to_vec()),
            Step::BulkOut(standard(cmd::VERIFY_DEVICE).to_vec()),
            Step::BulkIn {
                max: CSW_LEN,
                reply: csw(0),
            },
            Step::BulkOut(request(AW_USB_READ, 32).to_vec()),
            Step::BulkIn {
                max: 32,
                reply: verify_reply,
            },
            Step::BulkIn {
                max: CSW_LEN,
                reply: csw(0),
            },
        ]);
        let verify = Fel::new(&mut script).verify().await.unwrap();
        assert_eq!(verify.board, 0x0016_1000);
        assert!(script.finished());
    }

    #[tokio::test]
    async fn a_failed_status_byte_stops_the_command() {
        let mut csw = b"AWUS".to_vec();
        csw.extend_from_slice(&[0; 8]);
        csw.push(1);
        let mut script = Script::new(vec![
            Step::BulkOut(request(AW_USB_WRITE, 16).to_vec()),
            Step::BulkOut(message(cmd::RUN, 0x2000, 0).to_vec()),
            Step::BulkIn {
                max: CSW_LEN,
                reply: csw,
            },
        ]);
        let error = Fel::new(&mut script).run(0x2000).await.unwrap_err();
        assert!(matches!(error, UsbError::Protocol(text) if text.contains("status byte is 1")));
    }
}
