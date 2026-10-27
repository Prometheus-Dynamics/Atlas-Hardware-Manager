//! Atlas's side of the raze-flasher: a Raze booted over USB into the
//! flasher (`raze-flasher` boot files) shows up as `1d6b:0104` "Raze
//! flasher" with a vendor bulk interface. [`find`] and [`open`] reach it;
//! [`Flasher`] speaks the protocol (`atlas-flasher-proto`): `hello` binds
//! the session to the board, `write` streams the image as it is stored (the
//! board decompresses) with progress, `verify` reads it back, and `reboot`
//! starts the new system.

use std::io;
use std::time::Duration;

use async_trait::async_trait;
use atlas_flasher_proto::{
    Decoder, Emmc, Format, Kind, PRODUCT, PRODUCT_ID, PROTOCOL, Phase, RebootInto, Reply, Request,
    VENDOR_ID, data_frame, json_frame,
};
use nusb::DeviceInfo;
use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, In, Out};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

pub use atlas_flasher_proto::{Emmc as EmmcInfo, Format as ImageFormat};

/// How long a reply may take. Writing reports progress four times a
/// second; the longest quiet stretch is the final eMMC sync.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// Image bytes per data frame.
const CHUNK: usize = 512 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FlasherError {
    #[error("USB: {0}")]
    Usb(String),
    #[error("the flasher's link: {0}")]
    Io(#[from] io::Error),
    #[error("the flasher closed the link")]
    Closed,
    #[error("the flasher didn't answer within {0:?}")]
    Timeout(Duration),
    #[error("the flasher said: {0}")]
    Refused(String),
    #[error("the flasher replied out of turn: {0}")]
    Unexpected(String),
    #[error("this is board {found}, not {expected}")]
    WrongBoard { expected: String, found: String },
    #[error("the flasher speaks protocol {0}; this Atlas speaks {PROTOCOL}")]
    Protocol(u32),
    #[error("read back as {found}, written as {expected}")]
    VerifyMismatch { expected: String, found: String },
}

/// What the board said in `hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub daemon: String,
    pub serial: String,
    pub model: String,
    pub revision: Option<String>,
}

/// One progress report: `received` of `of` bytes as sent, and `written`
/// bytes on the eMMC (while verifying: read back, of `of`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub received: u64,
    pub of: u64,
    pub written: u64,
}

impl Progress {
    /// How far along, 0 to 1.
    pub fn fraction(&self) -> f64 {
        let done = match self.phase {
            Phase::Writing => self.received,
            Phase::Verifying => self.written,
        };
        if self.of == 0 {
            0.0
        } else {
            (done as f64 / self.of as f64).clamp(0.0, 1.0)
        }
    }
}

/// Where requests go. Messages end with [`end`](Self::end), so the board's
/// read returns at a message's end (a short or zero-length packet on USB).
#[async_trait]
pub trait Transport: Send {
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()>;
    async fn end(&mut self) -> io::Result<()>;
}

/// Any async writer; `end` flushes.
pub struct StreamTransport<W>(pub W);

#[async_trait]
impl<W: AsyncWrite + Send + Unpin> Transport for StreamTransport<W> {
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes).await
    }
    async fn end(&mut self) -> io::Result<()> {
        self.0.flush().await
    }
}

struct UsbTransport(nusb::io::EndpointWrite<Bulk>);

#[async_trait]
impl Transport for UsbTransport {
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes).await
    }
    async fn end(&mut self) -> io::Result<()> {
        self.0.flush_end_async().await
    }
}

/// Whether a USB device is a Raze flasher.
pub(crate) fn is_flasher(info: &DeviceInfo) -> bool {
    info.vendor_id() == VENDOR_ID
        && info.product_id() == PRODUCT_ID
        && info.product_string() == Some(PRODUCT)
}

/// The board serial a flasher reports as its USB serial (the boot ROM's,
/// the last 8 hex digits), lowercase.
pub fn board_serial(info: &DeviceInfo) -> Option<String> {
    let serial = info.serial_number()?.trim();
    let tail = serial.get(serial.len().checked_sub(8)?..)?;
    tail.chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| tail.to_ascii_lowercase())
}

/// Flashers plugged in now; with `serial`, only that board's.
pub async fn find(serial: Option<&str>) -> Result<Vec<DeviceInfo>, FlasherError> {
    let devices = nusb::list_devices()
        .await
        .map_err(|error| FlasherError::Usb(error.to_string()))?;
    Ok(devices
        .filter(is_flasher)
        .filter(|info| {
            serial.is_none_or(|want| {
                board_serial(info).is_some_and(|have| have.eq_ignore_ascii_case(want))
            })
        })
        .collect())
}

/// Waits up to `timeout` for a flasher (that board's, with `serial`).
pub async fn wait_for(
    serial: Option<&str>,
    timeout: Duration,
) -> Result<Option<DeviceInfo>, FlasherError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(info) = find(serial).await?.into_iter().next() {
            return Ok(Some(info));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Opens a flasher's vendor interface.
pub async fn open(info: &DeviceInfo) -> Result<Flasher, FlasherError> {
    let usb = |error: nusb::Error| FlasherError::Usb(error.to_string());
    let device = info.open().await.map_err(usb)?;
    let (number, endpoint_in, endpoint_out) = {
        let config = device
            .active_configuration()
            .map_err(|error| FlasherError::Usb(error.to_string()))?;
        let found = config.interface_alt_settings().find_map(|interface| {
            if interface.class() != atlas_flasher_proto::INTERFACE_CLASS {
                return None;
            }
            let bulk = |direction: Direction| {
                interface
                    .endpoints()
                    .find(|ep| {
                        ep.transfer_type() == TransferType::Bulk && ep.direction() == direction
                    })
                    .map(|ep| ep.address())
            };
            Some((
                interface.interface_number(),
                bulk(Direction::In)?,
                bulk(Direction::Out)?,
            ))
        });
        found.ok_or_else(|| FlasherError::Usb("the flasher has no bulk vendor interface".into()))?
    };
    #[cfg(target_os = "linux")]
    let claimed = device.detach_and_claim_interface(number).await;
    #[cfg(not(target_os = "linux"))]
    let claimed = device.claim_interface(number).await;
    let interface = claimed.map_err(usb)?;
    let to_board = interface
        .endpoint::<Bulk, Out>(endpoint_out)
        .map_err(|error| FlasherError::Usb(error.to_string()))?;
    let from_board = interface
        .endpoint::<Bulk, In>(endpoint_in)
        .map_err(|error| FlasherError::Usb(error.to_string()))?;
    let writer = to_board.writer(256 * 1024).with_num_transfers(4);
    let reader = from_board.reader(16 * 1024).with_num_transfers(2);
    Ok(Flasher::new(UsbTransport(writer), reader))
}

/// A session with one flasher.
pub struct Flasher {
    transport: Box<dyn Transport>,
    replies: mpsc::UnboundedReceiver<Result<Reply, FlasherError>>,
    reading: tokio::task::JoinHandle<()>,
}

impl Drop for Flasher {
    fn drop(&mut self) {
        self.reading.abort();
    }
}

impl Flasher {
    /// A session over any pipe: replies are read in the background, so the
    /// board's progress never waits behind the image Atlas is sending.
    pub fn new(
        transport: impl Transport + 'static,
        mut from_board: impl AsyncRead + Send + Unpin + 'static,
    ) -> Self {
        let (tx, replies) = mpsc::unbounded_channel();
        let reading = tokio::spawn(async move {
            let mut decoder = Decoder::default();
            let mut buffer = vec![0u8; 16 * 1024];
            loop {
                let n = match from_board.read(&mut buffer).await {
                    Ok(0) => {
                        let _ = tx.send(Err(FlasherError::Closed));
                        return;
                    }
                    Ok(n) => n,
                    Err(error) => {
                        let _ = tx.send(Err(error.into()));
                        return;
                    }
                };
                decoder.push(&buffer[..n]);
                while let Some(frame) = decoder.next_frame() {
                    let reply = match frame {
                        Ok(frame) if frame.kind == Kind::Json => {
                            serde_json::from_slice::<Reply>(&frame.payload)
                                .map_err(|error| FlasherError::Unexpected(error.to_string()))
                        }
                        Ok(_) => Err(FlasherError::Unexpected("data from the board".into())),
                        Err(error) => Err(FlasherError::Unexpected(error.to_string())),
                    };
                    if tx.send(reply).is_err() {
                        return;
                    }
                }
            }
        });
        Self {
            transport: Box::new(transport),
            replies,
            reading,
        }
    }

    async fn request(&mut self, request: &Request) -> Result<(), FlasherError> {
        self.transport.send(&json_frame(request)).await?;
        self.transport.end().await?;
        Ok(())
    }

    async fn reply(&mut self) -> Result<Reply, FlasherError> {
        match tokio::time::timeout(REPLY_TIMEOUT, self.replies.recv()).await {
            Err(_) => Err(FlasherError::Timeout(REPLY_TIMEOUT)),
            Ok(None) => Err(FlasherError::Closed),
            Ok(Some(reply)) => match reply? {
                Reply::Error { message } => Err(FlasherError::Refused(message)),
                reply => Ok(reply),
            },
        }
    }

    /// The next reply that isn't progress, reporting progress on the way.
    async fn outcome(
        &mut self,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<Reply, FlasherError> {
        loop {
            match self.reply().await? {
                Reply::Progress {
                    phase,
                    received,
                    of,
                    written,
                } => progress(Progress {
                    phase,
                    received,
                    of,
                    written,
                }),
                reply => return Ok(reply),
            }
        }
    }

    /// Opens the session; refuses a board that isn't `expect_serial` (its
    /// boot ROM serial, the last 8 hex digits of the full one).
    pub async fn hello(
        &mut self,
        atlas: &str,
        expect_serial: Option<&str>,
    ) -> Result<Hello, FlasherError> {
        self.request(&Request::Hello {
            protocol: PROTOCOL,
            atlas: atlas.into(),
        })
        .await?;
        match self.reply().await? {
            Reply::Hello {
                protocol,
                daemon,
                serial,
                model,
                revision,
            } => {
                if protocol != PROTOCOL {
                    return Err(FlasherError::Protocol(protocol));
                }
                if let Some(expected) = expect_serial {
                    let tail = serial
                        .get(serial.len().saturating_sub(8)..)
                        .unwrap_or(&serial);
                    if !tail.eq_ignore_ascii_case(expected) {
                        return Err(FlasherError::WrongBoard {
                            expected: expected.into(),
                            found: serial,
                        });
                    }
                }
                Ok(Hello {
                    daemon,
                    serial,
                    model,
                    revision,
                })
            }
            other => Err(FlasherError::Unexpected(format!("{other:?}"))),
        }
    }

    pub async fn info(&mut self) -> Result<Emmc, FlasherError> {
        self.request(&Request::Info).await?;
        match self.reply().await? {
            Reply::Info { emmc, .. } => Ok(emmc),
            other => Err(FlasherError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Streams `size` bytes of `image` and has the board write them.
    /// Returns the bytes written to the eMMC and their SHA-256.
    pub async fn write(
        &mut self,
        mut image: impl AsyncRead + Send + Unpin,
        size: u64,
        sha256: Option<String>,
        format: Format,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<(u64, String), FlasherError> {
        self.request(&Request::Write {
            size,
            sha256,
            format,
            keep_data: false,
        })
        .await?;
        match self.reply().await? {
            Reply::Ready => {}
            other => return Err(FlasherError::Unexpected(format!("{other:?}"))),
        }
        let mut buffer = vec![0u8; CHUNK];
        let mut sent: u64 = 0;
        let streamed: Result<(), FlasherError> = async {
            while sent < size {
                let n = image.read(&mut buffer).await?;
                if n == 0 {
                    return Err(FlasherError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("the image ended at {sent} of {size} bytes"),
                    )));
                }
                self.transport.send(&data_frame(&buffer[..n])).await?;
                sent += n as u64;
                // Progress and a refusal arrive while sending.
                while let Ok(reply) = self.replies.try_recv() {
                    match reply? {
                        Reply::Progress {
                            phase,
                            received,
                            of,
                            written,
                        } => progress(Progress {
                            phase,
                            received,
                            of,
                            written,
                        }),
                        Reply::Error { message } => return Err(FlasherError::Refused(message)),
                        other => return Err(FlasherError::Unexpected(format!("{other:?}"))),
                    }
                }
            }
            Ok(())
        }
        .await;
        // End the stream either way, so the board is ready for what's next.
        self.transport.send(&data_frame(&[])).await?;
        self.transport.end().await?;
        streamed?;
        match self.outcome(progress).await? {
            Reply::Written { bytes, sha256 } => Ok((bytes, sha256)),
            other => Err(FlasherError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Has the board read back what it wrote; errors when it differs.
    pub async fn verify(
        &mut self,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<String, FlasherError> {
        self.request(&Request::Verify).await?;
        match self.outcome(progress).await? {
            Reply::Verified { ok: true, sha256 } => Ok(sha256),
            Reply::Verified { ok: false, sha256 } => Err(FlasherError::VerifyMismatch {
                expected: "the written image".into(),
                found: sha256,
            }),
            other => Err(FlasherError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Reboots the board into its eMMC. The link goes away after.
    pub async fn reboot(&mut self) -> Result<(), FlasherError> {
        self.request(&Request::Reboot {
            into: RebootInto::Emmc,
        })
        .await?;
        match self.reply().await? {
            Reply::Ok => Ok(()),
            other => Err(FlasherError::Unexpected(format!("{other:?}"))),
        }
    }
}
