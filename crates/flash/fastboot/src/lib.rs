//! A fastboot client for the last step of a flasher start. Every family but
//! the Pi starts its flasher the same way: the boot ROM loader gets U-Boot
//! running from RAM, then U-Boot's own USB fastboot takes the flasher kernel
//! and initramfs into RAM and boots them. This crate is that last step.
//!
//! It speaks the fastboot wire protocol over [`atlas_usb`]'s [`UsbIo`] and
//! offers three commands: `getvar`, `download` and `boot`. There is no flash,
//! erase, reboot or OEM command, and no way to send any other command string:
//! the commands are a closed set in the crate.
//!
//! # What `boot` takes
//!
//! `boot` boots the image that `download` put in RAM. The image is the bytes
//! the caller passes; this crate does not build one. U-Boot's `fastboot boot`
//! takes an Android boot image (the `ANDROID!` header with the kernel and the
//! ramdisk in it), which is the form this crate expects to be given. Which
//! header version, page size and load addresses the flasher's U-Boot accepts
//! is not settled here: the image is built by whoever builds the flasher
//! product (B9, D6). Building it in this crate is open until that is known.
//!
//! # The flow
//!
//! 1. [`Fastboot::open`] finds the device at its location, waiting for it to
//!    come up (U-Boot re-enumerates when it starts).
//! 2. [`Fastboot::getvar`] reads [`Variable::MaxDownloadSize`].
//! 3. [`Fastboot::download`] sends the image. It is refused, before anything
//!    is sent, when it is larger than the device's limit. There is no
//!    splitting.
//! 4. [`Fastboot::boot`] boots it. The device re-enumerates when the kernel
//!    starts, so a disconnect after the device accepted `boot` is success.
//!    A disconnect before it is an error.
//!
//! Every transfer has a time limit. A stall or a timeout is an error the
//! caller can retry by opening the device again. USB ids and interface
//! choice come from the caller: [`find`] matches the ids it is given.
//!
//! # Protocol source
//!
//! Written from the fastboot protocol's public description (command and
//! reply codes, the `download` data phase, the hex `max-download-size`). No
//! code was copied from AOSP or any other project. Atlas is MIT.
//!
//! # Status
//!
//! The protocol is tested against [`atlas_usb::sim::SimBus`]. The hardware
//! path is untested on board.

mod error;
mod protocol;

use std::fmt;
use std::time::Duration;

use atlas_usb::{DeviceSummary, OpenOptions, UsbBus, UsbError, UsbId, UsbIo, await_device};
use tokio::time::timeout;

pub use error::FastbootError;
pub use protocol::Variable;
use protocol::{COMMAND_MAX, Command, REPLY_MAX, Reply};

/// The largest chunk written in one bulk transfer during a download. Each
/// chunk has its own time limit, so a slow link does not fail on one big
/// write.
const DATA_CHUNK: usize = 1 << 20;

/// How many `INFO` lines may come before one reply. A device that sends more
/// is not following the protocol.
const INFO_MAX: usize = 256;

/// The backstop time limit for one transfer. The USB backend has its own
/// shorter limits from [`OpenOptions`]; this one only guarantees that a
/// transfer ends even on a backend that never times out.
pub const TRANSFER_LIMIT: Duration = Duration::from_secs(30);

/// An open fastboot device.
pub struct Fastboot {
    io: Box<dyn UsbIo>,
    limit: Duration,
    staged: bool,
}

impl fmt::Debug for Fastboot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fastboot")
            .field("limit", &self.limit)
            .field("staged", &self.staged)
            .finish_non_exhaustive()
    }
}

impl Fastboot {
    /// Wraps an open device. `limit` is the backstop time limit for each
    /// transfer.
    pub fn new(io: Box<dyn UsbIo>, limit: Duration) -> Self {
        Self {
            io,
            limit,
            staged: false,
        }
    }

    /// Opens the device at `location` and claims its interface as `options`
    /// says. Waits up to `wait` for the device to appear, because U-Boot
    /// re-enumerates when it starts.
    pub async fn open(
        bus: &dyn UsbBus,
        location: &str,
        options: &OpenOptions,
        wait: Duration,
    ) -> Result<Self, FastbootError> {
        let io = await_device(bus, location, options, wait).await?;
        Ok(Self::new(io, TRANSFER_LIMIT))
    }

    /// Reads one variable. The value is the device's text, as it reports it.
    pub async fn getvar(&mut self, variable: Variable) -> Result<String, FastbootError> {
        self.send(Command::GetVar(variable)).await?;
        let mut info = Vec::new();
        match self.reply(Command::GetVar(variable), &mut info).await? {
            Reply::Okay(value) => Ok(value),
            other => Err(unexpected("getvar", &other)),
        }
    }

    /// Sends `image` to the device, which holds it in RAM for `boot`. Returns
    /// the device's `INFO` lines. Refused before anything is sent when the
    /// image is larger than the device's `max-download-size`.
    pub async fn download(&mut self, image: &[u8]) -> Result<Vec<String>, FastbootError> {
        self.staged = false;
        if image.is_empty() {
            return Err(FastbootError::EmptyImage);
        }
        let max = self.max_download_size().await?;
        let size = image.len() as u64;
        let limit = max.min(u64::from(u32::MAX));
        if size > limit {
            return Err(FastbootError::ImageTooLarge { size, max: limit });
        }
        let announced = size as u32;

        self.send(Command::Download(announced)).await?;
        let mut info = Vec::new();
        match self.reply(Command::Download(announced), &mut info).await? {
            Reply::Data(accepted) if accepted == announced => {}
            Reply::Data(accepted) => {
                return Err(FastbootError::Protocol(format!(
                    "the device expects {accepted} bytes, not {announced}"
                )));
            }
            other => return Err(unexpected("download", &other)),
        }

        self.send_data(image).await?;
        match self.reply(Command::Download(announced), &mut info).await? {
            Reply::Okay(_) => {
                self.staged = true;
                Ok(info)
            }
            other => Err(unexpected("download", &other)),
        }
    }

    /// Boots the image from [`Fastboot::download`]. Returns the device's
    /// `INFO` lines once it accepts the command.
    ///
    /// Acceptance is the device's `OKAY`. After it the device starts the
    /// kernel and leaves the bus, which is the success path: nothing more is
    /// read, so the disconnect that follows is not an error. A disconnect or
    /// a stall before `OKAY` is an error.
    pub async fn boot(mut self) -> Result<Vec<String>, FastbootError> {
        if !self.staged {
            return Err(FastbootError::NotStaged);
        }
        self.send(Command::Boot).await?;
        let mut info = Vec::new();
        match self.reply(Command::Boot, &mut info).await? {
            Reply::Okay(_) => Ok(info),
            other => Err(unexpected("boot", &other)),
        }
    }

    async fn max_download_size(&mut self) -> Result<u64, FastbootError> {
        let text = self.getvar(Variable::MaxDownloadSize).await?;
        protocol::parse_size(&text).ok_or_else(|| {
            FastbootError::Protocol(format!("max-download-size is not hex: {text:?}"))
        })
    }

    async fn send(&mut self, command: Command) -> Result<(), FastbootError> {
        let bytes = protocol::encode(command);
        if bytes.len() > COMMAND_MAX {
            return Err(FastbootError::Protocol(format!(
                "the {} command is {} bytes, over the {COMMAND_MAX}-byte limit",
                command.name(),
                bytes.len()
            )));
        }
        bounded(self.limit, "the command", self.io.bulk_out(&bytes)).await?;
        Ok(())
    }

    /// The image itself, in chunks. A write that fills its last packet exactly
    /// is followed by a zero-length packet, as the USB layer requires.
    async fn send_data(&mut self, image: &[u8]) -> Result<(), FastbootError> {
        for chunk in image.chunks(DATA_CHUNK) {
            bounded(self.limit, "the image data", self.io.bulk_out(chunk)).await?;
        }
        let packet = self.io.max_packet_out();
        if packet > 0 && image.len().is_multiple_of(packet) {
            bounded(self.limit, "the image data", self.io.bulk_out(&[])).await?;
        }
        Ok(())
    }

    /// Reads replies until one that is not `INFO`. `INFO` lines are kept and
    /// logged. `FAIL` is returned as [`FastbootError::Refused`].
    async fn reply(
        &mut self,
        command: Command,
        info: &mut Vec<String>,
    ) -> Result<Reply, FastbootError> {
        for _ in 0..INFO_MAX {
            let bytes = bounded(self.limit, "a reply", self.io.bulk_in(REPLY_MAX)).await?;
            match protocol::parse(&bytes).map_err(FastbootError::Protocol)? {
                Reply::Info(text) => {
                    tracing::info!(command = command.name(), "fastboot: {text}");
                    info.push(text);
                }
                Reply::Fail(message) => {
                    return Err(FastbootError::Refused {
                        command: command.name(),
                        message,
                    });
                }
                other => return Ok(other),
            }
        }
        Err(FastbootError::Protocol(format!(
            "more than {INFO_MAX} INFO lines before the reply to {}",
            command.name()
        )))
    }
}

/// Runs one transfer under the backstop time limit.
async fn bounded<T>(
    limit: Duration,
    what: &str,
    transfer: impl std::future::Future<Output = Result<T, UsbError>>,
) -> Result<T, UsbError> {
    match timeout(limit, transfer).await {
        Ok(result) => result,
        Err(_) => Err(UsbError::Timeout(what.to_string())),
    }
}

fn unexpected(command: &str, reply: &Reply) -> FastbootError {
    let code = match reply {
        Reply::Okay(_) => "OKAY",
        Reply::Fail(_) => "FAIL",
        Reply::Info(_) => "INFO",
        Reply::Data(_) => "DATA",
    };
    FastbootError::Protocol(format!("an unexpected {code} reply to {command}"))
}

/// The first device on the bus whose vendor and product id is one of `ids`.
/// The caller owns the ids; this crate has none of its own.
pub async fn find(bus: &dyn UsbBus, ids: &[UsbId]) -> Result<Option<DeviceSummary>, UsbError> {
    Ok(bus
        .devices()
        .await?
        .into_iter()
        .find(|device| ids.iter().any(|id| id.matches(device))))
}
