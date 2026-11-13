//! A claimed vendor interface's two bulk endpoints as a byte link.
//!
//! [`open_link`] finds the interface by its string, checks it has exactly one
//! bulk IN and one bulk OUT endpoint, claims it, and splits it into a
//! [`LinkReader`] and a [`LinkWriter`]. Each half can be moved to its own
//! task or thread, and each is a byte stream: USB packet boundaries are not
//! visible to the caller, except where [`ZeroLength::OnFlush`] asks for them.
//!
//! The primary API is async: [`LinkReader`] implements tokio's `AsyncRead` and
//! [`LinkWriter`] its `AsyncWrite`. nusb's transfers are async, and the
//! existing [`UsbIo`](crate::UsbIo) is async, so the blocking forms wrap the
//! same halves. [`Link::into_blocking`] gives [`BlockingReader`] and
//! [`BlockingWriter`], which implement `std::io::Read` and `std::io::Write`.
//! Each one owns a current-thread tokio runtime, created by that call (not
//! spawned, and no thread is started). A blocking call must not be made from
//! inside a tokio runtime on the same thread: the runtime refuses to block
//! there.
//!
//! Unplug: both halves fail with an error whose [`is_device_gone`] is true.
//! That error ends a half for good; open the link again to continue.
//! To stop a blocked blocking read, drop the blocking write half.

use std::fmt;
use std::future::Future;
use std::io::{self, Read, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::Notify;

use crate::{
    BulkRead, BulkWrite, EndpointDirection, EndpointKind, EndpointSummary, InterfaceSummary,
    OpenOptions, Timeouts, UsbBus, UsbError,
};

/// IN transfers kept queued on the device. Each asks for one packet, so a
/// read returns as soon as a packet arrives. Eight keeps the host asking
/// ahead of the device's next packet; the memory cost is eight packets.
pub const READ_IN_FLIGHT: usize = 8;

/// Bytes in one OUT transfer. Sixteen KiB is a multiple of every USB bulk
/// packet size (8 to 1024 bytes), so each transfer is whole packets except
/// the last one of a write, and it keeps per-transfer overhead small.
pub const WRITE_TRANSFER: usize = 16 * 1024;

/// OUT transfers queued at once. Four lets a write go on while three are
/// still on their way, and leaves room for a zero-length packet beside them.
/// The memory bound is four transfers, 64 KiB.
pub const WRITE_IN_FLIGHT: usize = 4;

/// When a write sends a zero-length packet. A device-side byte stream reads a
/// zero-length packet as end-of-stream, so the default never sends one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZeroLength {
    /// Never. A write that ends on a full packet is left open on the wire
    /// until the next short packet; the peer must not rely on transfer ends.
    /// This is right for a byte-stream peer such as a FunctionFS function.
    #[default]
    Never,
    /// On [`flush`](tokio::io::AsyncWriteExt::flush) (and shutdown), when the
    /// last bytes written ended on a full packet, send a zero-length packet
    /// so a transfer-reading peer's read returns. Only for a peer that reads
    /// by transfer.
    OnFlush,
}

/// How a link waits. Reads wait for the device by default, because a board
/// may be quiet for as long as it likes; writes time out after the bulk
/// timeout, because a device that takes nothing for that long is wedged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkOptions {
    pub zero_length: ZeroLength,
    /// `None` waits while the device stays on the bus. A read that times out
    /// loses no bytes and may be retried.
    pub read_timeout: Option<Duration>,
    /// `None` waits. A write that times out leaves the byte stream in an
    /// unknown state, so the writer then fails every later call with the same
    /// error.
    pub write_timeout: Option<Duration>,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            zero_length: ZeroLength::Never,
            read_timeout: None,
            write_timeout: Some(Timeouts::default().bulk),
        }
    }
}

/// The error the device is gone with: the kind is `NotConnected`, and
/// [`usb_error`] returns [`UsbError::Disconnected`].
pub fn is_device_gone(error: &io::Error) -> bool {
    matches!(usb_error(error), Some(UsbError::Disconnected))
}

/// The [`UsbError`] behind a link's `io::Error`, when it has one.
pub fn usb_error(error: &io::Error) -> Option<&UsbError> {
    let inner: &(dyn std::error::Error + Send + Sync) = error.get_ref()?;
    inner.downcast_ref::<UsbError>()
}

fn to_io(error: UsbError) -> io::Error {
    let kind = match &error {
        UsbError::Disconnected => io::ErrorKind::NotConnected,
        UsbError::Timeout(_) => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

fn lost_endpoint() -> io::Error {
    io::Error::other("the link has lost its endpoint")
}

type ReadFuture =
    Pin<Box<dyn Future<Output = (Box<dyn BulkRead>, Result<Vec<u8>, UsbError>)> + Send>>;
type WriteFuture = Pin<Box<dyn Future<Output = (Box<dyn BulkWrite>, Result<(), UsbError>)> + Send>>;

fn read_future(mut half: Box<dyn BulkRead>) -> ReadFuture {
    Box::pin(async move {
        let result = half.complete().await;
        (half, result)
    })
}

fn write_future(mut half: Box<dyn BulkWrite>) -> WriteFuture {
    Box::pin(async move {
        let result = half.complete().await;
        (half, result)
    })
}

/// What the reader holds: the endpoint half, bytes received and not yet
/// handed out, and whether the device is gone.
struct ReadCore {
    half: Option<Box<dyn BulkRead>>,
    received: Vec<u8>,
    at: usize,
    gone: bool,
}

impl ReadCore {
    fn new(half: Box<dyn BulkRead>) -> Self {
        Self {
            half: Some(half),
            received: Vec::new(),
            at: 0,
            gone: false,
        }
    }

    fn has_received(&self) -> bool {
        self.at < self.received.len()
    }

    fn take_received(&mut self, into: &mut [u8]) -> usize {
        let n = into.len().min(self.received.len() - self.at);
        into[..n].copy_from_slice(&self.received[self.at..self.at + n]);
        self.at += n;
        if self.at == self.received.len() {
            self.received.clear();
            self.at = 0;
        }
        n
    }

    fn set_received(&mut self, bytes: Vec<u8>) {
        self.received = bytes;
        self.at = 0;
    }

    /// Ends the half for good on the gone kind; any other failure is reported
    /// once and the next read tries again.
    fn fail(&mut self, error: UsbError) -> io::Error {
        if error == UsbError::Disconnected {
            self.gone = true;
        }
        to_io(error)
    }

    fn check_gone(&self) -> io::Result<()> {
        if self.gone {
            Err(to_io(UsbError::Disconnected))
        } else {
            Ok(())
        }
    }
}

fn queue_reads(half: &mut dyn BulkRead) {
    while half.pending() < READ_IN_FLIGHT {
        half.submit();
    }
}

/// What the writer holds. A failed write is sticky: the byte stream position
/// is unknown after a stall, a timeout or an unplug, so the caller must open
/// the link again.
struct WriteCore {
    half: Option<Box<dyn BulkWrite>>,
    zero_length: ZeroLength,
    ends_full: bool,
    failed: Option<UsbError>,
}

impl WriteCore {
    fn new(half: Box<dyn BulkWrite>, zero_length: ZeroLength) -> Self {
        Self {
            half: Some(half),
            zero_length,
            ends_full: false,
            failed: None,
        }
    }

    fn check(&self) -> io::Result<()> {
        match &self.failed {
            Some(error) => Err(to_io(error.clone())),
            None => Ok(()),
        }
    }

    fn fail(&mut self, error: UsbError) -> io::Error {
        self.failed = Some(error.clone());
        to_io(error)
    }

    fn pending(&self) -> usize {
        self.half.as_ref().map_or(0, |half| half.pending())
    }

    fn has_slot(&self) -> bool {
        self.half
            .as_ref()
            .is_some_and(|half| half.pending() < WRITE_IN_FLIGHT)
    }

    fn zero_length_due(&self) -> bool {
        self.zero_length == ZeroLength::OnFlush && self.ends_full && self.has_slot()
    }

    /// Queues one OUT transfer. The caller has checked there is a slot.
    fn submit(&mut self, data: Vec<u8>) {
        if let Some(half) = self.half.as_mut() {
            let packet = half.max_packet().max(1);
            self.ends_full = !data.is_empty() && data.len().is_multiple_of(packet);
            half.submit(data);
        }
    }
}

/// The read half of a claimed link, as an `AsyncRead`. A read returns the
/// bytes of one packet or more, as soon as they arrive; it never returns
/// `Ok(0)` for a non-empty buffer, so a zero-length packet is not the end.
pub struct LinkReader {
    core: ReadCore,
    busy: Option<ReadFuture>,
}

impl fmt::Debug for LinkReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkReader")
            .field("gone", &self.core.gone)
            .finish_non_exhaustive()
    }
}

impl LinkReader {
    fn new(half: Box<dyn BulkRead>) -> Self {
        Self {
            core: ReadCore::new(half),
            busy: None,
        }
    }
}

impl AsyncRead for LinkReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if this.core.has_received() {
                let n = this.core.take_received(buf.initialize_unfilled());
                buf.advance(n);
                return Poll::Ready(Ok(()));
            }
            if let Err(error) = this.core.check_gone() {
                return Poll::Ready(Err(error));
            }
            if let Some(busy) = this.busy.as_mut() {
                let (half, result) = ready!(busy.as_mut().poll(cx));
                this.busy = None;
                this.core.half = Some(half);
                match result {
                    // An empty packet is handled by the loop: it asks again.
                    Ok(bytes) => this.core.set_received(bytes),
                    Err(error) => return Poll::Ready(Err(this.core.fail(error))),
                }
                continue;
            }
            let Some(mut half) = this.core.half.take() else {
                return Poll::Ready(Err(lost_endpoint()));
            };
            queue_reads(half.as_mut());
            this.busy = Some(read_future(half));
        }
    }
}

/// The write half of a claimed link, as an `AsyncWrite`. A write is accepted
/// once its transfer is queued, and waits while [`WRITE_IN_FLIGHT`] are
/// already queued. `flush` waits until every queued transfer has completed.
/// A write error is reported by the call that meets it and every call after.
pub struct LinkWriter {
    core: WriteCore,
    busy: Option<WriteFuture>,
}

impl fmt::Debug for LinkWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkWriter")
            .field("failed", &self.core.failed)
            .finish_non_exhaustive()
    }
}

impl LinkWriter {
    fn new(half: Box<dyn BulkWrite>, zero_length: ZeroLength) -> Self {
        Self {
            core: WriteCore::new(half, zero_length),
            busy: None,
        }
    }
}

impl AsyncWrite for LinkWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Err(error) = this.core.check() {
            return Poll::Ready(Err(error));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            if let Some(busy) = this.busy.as_mut() {
                let (half, result) = ready!(busy.as_mut().poll(cx));
                this.busy = None;
                this.core.half = Some(half);
                if let Err(error) = result {
                    return Poll::Ready(Err(this.core.fail(error)));
                }
                continue;
            }
            if this.core.has_slot() {
                let n = buf.len().min(WRITE_TRANSFER);
                this.core.submit(buf[..n].to_vec());
                return Poll::Ready(Ok(n));
            }
            let Some(half) = this.core.half.take() else {
                return Poll::Ready(Err(lost_endpoint()));
            };
            this.busy = Some(write_future(half));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if let Err(error) = this.core.check() {
                return Poll::Ready(Err(error));
            }
            if let Some(busy) = this.busy.as_mut() {
                let (half, result) = ready!(busy.as_mut().poll(cx));
                this.busy = None;
                this.core.half = Some(half);
                if let Err(error) = result {
                    return Poll::Ready(Err(this.core.fail(error)));
                }
                continue;
            }
            if this.core.zero_length_due() {
                this.core.submit(Vec::new());
                continue;
            }
            if this.core.pending() == 0 {
                return Poll::Ready(Ok(()));
            }
            let Some(half) = this.core.half.take() else {
                return Poll::Ready(Err(lost_endpoint()));
            };
            this.busy = Some(write_future(half));
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // The link stays open: closing it means dropping the halves.
        self.poll_flush(cx)
    }
}

/// Stops a blocked read. The two blocking halves of one link share it: the
/// write half sets it when dropped, and the read half, blocked in a transfer,
/// stops at once and cancels what it had queued.
#[derive(Debug, Default)]
struct Closer {
    closed: AtomicBool,
    wake: Notify,
}

impl Closer {
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    async fn closed(&self) {
        loop {
            // Made before the check, so a close between the check and the
            // wait is not missed.
            let woken = self.wake.notified();
            if self.is_closed() {
                return;
            }
            woken.await;
        }
    }
}

fn stopped_error() -> io::Error {
    io::Error::other("the write half was dropped, so the read was stopped")
}

fn runtime() -> io::Result<Runtime> {
    Builder::new_current_thread().enable_time().build()
}

// LOCK: Orion F12a LinkRead/LinkWrite
/// The read side of a link, as a session moves it onto its own thread.
/// Provisional: it mirrors Orion's trait until the lock has it, and the
/// blocking read half of this crate implements it.
///
/// `read` blocks until bytes arrive. `Ok(0)` means nothing yet (a zero-length
/// packet), never the end of the stream. A device that is gone is an error
/// whose [`is_device_gone`] is true. To stop a blocked `read`, drop the write
/// half: the read returns an error at once, cancels what it had queued, and
/// the interface is released when both halves are gone.
pub trait LinkRead: Send + 'static {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize>;
}

// LOCK: Orion F12a LinkRead/LinkWrite
/// The write side of a link. Provisional, as [`LinkRead`].
///
/// `write_all` returns once every byte has been handed to the device, or
/// fails. It may be called from another thread than the reader's.
pub trait LinkWrite: Send + 'static {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
}

/// The blocking read half: `std::io::Read` and [`LinkRead`]. It owns a
/// current-thread tokio runtime, created by [`Link::into_blocking`]; its
/// blocking waits are a `select!` on the transfer and a wake-up, not a poll.
pub struct BlockingReader {
    core: ReadCore,
    runtime: Runtime,
    closer: Arc<Closer>,
}

impl fmt::Debug for BlockingReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockingReader")
            .field("gone", &self.core.gone)
            .finish_non_exhaustive()
    }
}

impl BlockingReader {
    /// One completed IN transfer: its bytes, possibly none. Blocks until one
    /// completes, the device goes, or the write half is dropped.
    fn next_packet(&mut self) -> io::Result<Vec<u8>> {
        if self.closer.is_closed() {
            return Err(stopped_error());
        }
        self.core.check_gone()?;
        let Some(half) = self.core.half.as_mut() else {
            return Err(lost_endpoint());
        };
        queue_reads(half.as_mut());
        let closer = Arc::clone(&self.closer);
        let outcome = self.runtime.block_on(async {
            tokio::select! {
                biased;
                () = closer.closed() => None,
                result = half.complete() => Some(result),
            }
        });
        match outcome {
            Some(Ok(bytes)) => Ok(bytes),
            Some(Err(error)) => Err(self.core.fail(error)),
            None => {
                if let Some(half) = self.core.half.as_mut() {
                    self.runtime.block_on(half.cancel());
                }
                Err(stopped_error())
            }
        }
    }
}

impl LinkRead for BlockingReader {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        if into.is_empty() {
            return Ok(0);
        }
        if !self.core.has_received() {
            let bytes = self.next_packet()?;
            self.core.set_received(bytes);
        }
        Ok(self.core.take_received(into))
    }
}

impl Read for BlockingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // A zero-length packet is not the end: ask again.
        loop {
            let n = LinkRead::read(self, buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
        }
    }
}

/// The blocking write half: `std::io::Write` and [`LinkWrite`]. Dropping it
/// stops a read the other half is blocked in.
pub struct BlockingWriter {
    core: WriteCore,
    runtime: Runtime,
    closer: Arc<Closer>,
}

impl fmt::Debug for BlockingWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockingWriter")
            .field("failed", &self.core.failed)
            .finish_non_exhaustive()
    }
}

impl Drop for BlockingWriter {
    fn drop(&mut self) {
        self.closer.close();
    }
}

impl BlockingWriter {
    fn complete_one(&mut self) -> io::Result<()> {
        let Some(half) = self.core.half.as_mut() else {
            return Err(lost_endpoint());
        };
        match self.runtime.block_on(half.complete()) {
            Ok(()) => Ok(()),
            Err(error) => Err(self.core.fail(error)),
        }
    }
}

impl LinkWrite for BlockingWriter {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        Write::write_all(self, bytes)?;
        Write::flush(self)
    }
}

impl Write for BlockingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.core.check()?;
        if buf.is_empty() {
            return Ok(0);
        }
        while !self.core.has_slot() {
            self.complete_one()?;
        }
        let n = buf.len().min(WRITE_TRANSFER);
        self.core.submit(buf[..n].to_vec());
        Ok(n)
    }

    /// Returns once every queued transfer has completed, or fails.
    fn flush(&mut self) -> io::Result<()> {
        loop {
            self.core.check()?;
            if self.core.zero_length_due() {
                self.core.submit(Vec::new());
                continue;
            }
            if self.core.pending() == 0 {
                return Ok(());
            }
            self.complete_one()?;
        }
    }
}

/// A claimed link before it is split or made blocking.
pub struct Link {
    reader: LinkReader,
    writer: LinkWriter,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

impl Link {
    /// The two independent async halves, for two tasks or threads.
    pub fn into_parts(self) -> (LinkReader, LinkWriter) {
        (self.reader, self.writer)
    }

    /// Both halves, blocking. Each half gets its own current-thread runtime,
    /// created by this call; no thread is started. Call it from outside any
    /// tokio runtime.
    pub fn into_blocking(self) -> io::Result<(BlockingReader, BlockingWriter)> {
        let closer = Arc::new(Closer::default());
        let reader = BlockingReader {
            core: self.reader.core,
            runtime: runtime()?,
            closer: Arc::clone(&closer),
        };
        let writer = BlockingWriter {
            core: self.writer.core,
            runtime: runtime()?,
            closer,
        };
        Ok((reader, writer))
    }
}

/// Finds the interface whose string is `string` on the device at `location`,
/// checks that it has exactly one bulk IN and one bulk OUT endpoint, claims
/// it, and returns the link. Refusals name what was found, and nothing is
/// claimed. `options.interface` is ignored: the interface comes from `string`.
pub async fn open_link(
    bus: &dyn UsbBus,
    location: &str,
    string: &str,
    options: &OpenOptions,
    link: LinkOptions,
) -> Result<Link, UsbError> {
    let interfaces = bus.interfaces(location, options.timeouts).await?;
    let interface = interfaces
        .iter()
        .find(|interface| interface.string.as_deref() == Some(string))
        .ok_or_else(|| UsbError::NotFound(format!("interface \"{string}\" at {location}")))?;
    check_endpoints(interface, location, string)?;
    let number = interface.number;
    let io = bus
        .open(
            location,
            &OpenOptions {
                interface: Some(number),
                timeouts: options.timeouts,
            },
        )
        .await?;
    let (reader, writer) = io.into_halves(&link)?;
    Ok(Link {
        reader: LinkReader::new(reader),
        writer: LinkWriter::new(writer, link.zero_length),
    })
}

fn check_endpoints(
    interface: &InterfaceSummary,
    location: &str,
    string: &str,
) -> Result<(), UsbError> {
    let count = |kind: EndpointKind, direction: EndpointDirection| {
        interface
            .endpoints
            .iter()
            .filter(|ep| ep.kind == kind && ep.direction == direction)
            .count()
    };
    let bulk_in = count(EndpointKind::Bulk, EndpointDirection::In);
    let bulk_out = count(EndpointKind::Bulk, EndpointDirection::Out);
    if interface.endpoints.len() == 2 && bulk_in == 1 && bulk_out == 1 {
        return Ok(());
    }
    Err(UsbError::Protocol(format!(
        "interface \"{string}\" at {location} has {}; a byte link needs exactly one bulk IN and one bulk OUT endpoint",
        describe_endpoints(&interface.endpoints)
    )))
}

fn describe_endpoints(endpoints: &[EndpointSummary]) -> String {
    if endpoints.is_empty() {
        return "no endpoints".into();
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for endpoint in endpoints {
        let kind = match endpoint.kind {
            EndpointKind::Control => "control",
            EndpointKind::Isochronous => "isochronous",
            EndpointKind::Bulk => "bulk",
            EndpointKind::Interrupt => "interrupt",
        };
        let direction = match endpoint.direction {
            EndpointDirection::In => "IN",
            EndpointDirection::Out => "OUT",
        };
        let label = format!("{kind} {direction}");
        match counts.iter_mut().find(|(name, _)| *name == label) {
            Some((_, n)) => *n += 1,
            None => counts.push((label, 1)),
        }
    }
    counts
        .iter()
        .map(|(label, n)| format!("{n} {label}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_wait_for_reads_and_time_out_writes() {
        let options = LinkOptions::default();
        assert_eq!(options.zero_length, ZeroLength::Never);
        assert_eq!(options.read_timeout, None);
        assert_eq!(options.write_timeout, Some(Timeouts::default().bulk));
    }

    #[test]
    fn a_gone_error_is_recognised_and_others_are_not() {
        assert!(is_device_gone(&to_io(UsbError::Disconnected)));
        assert!(!is_device_gone(&to_io(UsbError::Timeout(
            "a bulk read".into()
        ))));
        assert!(!is_device_gone(&to_io(UsbError::Stall)));
        let gone = to_io(UsbError::Disconnected);
        assert_eq!(gone.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn a_refusal_names_what_was_found() {
        let interface = InterfaceSummary::new(4, 0xff, 0, 0)
            .with_string("orion-link")
            .with_endpoint(EndpointSummary::new(0x81, EndpointKind::Bulk))
            .with_endpoint(EndpointSummary::new(0x82, EndpointKind::Bulk))
            .with_endpoint(EndpointSummary::new(0x01, EndpointKind::Bulk));
        let error = check_endpoints(&interface, "1-2", "orion-link").unwrap_err();
        let UsbError::Protocol(message) = error else {
            panic!("expected a protocol refusal");
        };
        assert!(message.contains("2 bulk IN, 1 bulk OUT"), "{message}");
    }
}
