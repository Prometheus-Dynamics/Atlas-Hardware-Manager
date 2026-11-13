//! The FEL client: each FEL command as the transfers the protocol makes
//! over a [`UsbIo`]. A command is a request block, the command body, then
//! the device's status block. A data stage is a request block for its
//! direction, the bytes, then the status block; a read's bytes come before
//! its status block. Every command's status byte must be zero. Wire format
//! and commands: [`crate::protocol`].

use atlas_usb::{UsbError, UsbIo};

use crate::protocol::{
    AW_USB_READ, AW_USB_WRITE, CSW_LEN, MAX_TRANSFER, STATUS_LEN, Status, VERIFY_LEN, Verify, cmd,
    message, parse_csw, parse_status, parse_verify, request, standard,
};
use crate::trace;

/// One FEL session over a download-mode device.
pub struct Fel<'a> {
    io: &'a mut dyn UsbIo,
}

impl std::fmt::Debug for Fel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fel").finish_non_exhaustive()
    }
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
        let bytes = self.read_data("verify device", VERIFY_LEN).await?;
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
    use crate::protocol::MODE_FEL;
    use atlas_usb::sim::{Answer, SimBus};
    use atlas_usb::{DeviceSummary, OpenOptions, Transfer, UsbBus};

    fn csw(status: u8) -> Vec<u8> {
        let mut csw = b"AWUS".to_vec();
        csw.extend_from_slice(&[0; 8]);
        csw.push(status);
        csw
    }

    fn verify_reply(mode: u16) -> Vec<u8> {
        let mut reply = vec![0u8; VERIFY_LEN];
        reply[0..8].copy_from_slice(b"AWUSBFEX");
        reply[8..12].copy_from_slice(&0x0016_1000u32.to_le_bytes());
        reply[16..18].copy_from_slice(&mode.to_le_bytes());
        reply
    }

    /// A SimBus with one device at `1-1`, opened as a handle.
    async fn open(bus: &SimBus) -> Box<dyn UsbIo> {
        bus.attach(DeviceSummary::new("1-1", 0x1f3a, 0xefe8));
        bus.open("1-1", &OpenOptions::default()).await.unwrap()
    }

    #[tokio::test]
    async fn verify_is_a_standard_command_then_a_32_byte_read() {
        let bus = SimBus::new();
        let mut io = open(&bus).await;
        bus.script(
            "1-1",
            [
                Answer::Ok,
                Answer::Ok,
                Answer::Data(csw(0)),
                Answer::Ok,
                Answer::Data(verify_reply(MODE_FEL)),
                Answer::Data(csw(0)),
            ],
        );
        let verify = Fel::new(io.as_mut()).verify().await.unwrap();
        assert_eq!(verify.board, 0x0016_1000);
        assert_eq!(verify.mode, MODE_FEL);
        // The bytes the host sent, in order: the command's request block and
        // body, then the read's request block.
        let sent: Vec<Vec<u8>> = bus
            .records("1-1")
            .into_iter()
            .filter_map(|record| match record.transfer {
                Transfer::BulkOut(data) => Some(data),
                _ => None,
            })
            .collect();
        assert_eq!(sent[0], request(AW_USB_WRITE, 16).to_vec());
        assert_eq!(sent[1], standard(cmd::VERIFY_DEVICE).to_vec());
        assert_eq!(sent[2], request(AW_USB_READ, 32).to_vec());
    }

    #[tokio::test]
    async fn a_failed_status_byte_stops_the_command() {
        let bus = SimBus::new();
        let mut io = open(&bus).await;
        bus.script("1-1", [Answer::Ok, Answer::Ok, Answer::Data(csw(1))]);
        let error = Fel::new(io.as_mut()).run(0x2000).await.unwrap_err();
        assert!(matches!(error, UsbError::Protocol(text) if text.contains("status byte is 1")));
    }
}
