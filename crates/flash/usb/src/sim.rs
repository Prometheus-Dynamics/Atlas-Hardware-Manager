//! A scriptable USB device for tests. A [`SimBus`] holds devices at their
//! locations; a loader opens one and drives it through [`UsbIo`] exactly as it
//! would a real one.
//!
//! Every transfer needs an answer. A device's answers are queued with
//! [`SimBus::script`] and used in order, one per transfer. A transfer with no
//! answer left fails, so a test cannot pass by accident. Each answer is one
//! of the ways a real device misbehaves: a stall, a timeout, a short read or
//! write, a protocol garbage reply, or a disconnect. A disconnect removes the
//! device, as a boot ROM reset does. Handles opened before it stay dead, even
//! after the device comes back at the same location, because that is a new
//! enumeration. Open again to use it.

use async_trait::async_trait;
use futures_core::Stream;
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::{Notify, mpsc};

use crate::bus::UsbBus;
use crate::gauge::{Gauges, Kind, Tracked};
use crate::hotplug::HotplugEvent;
use crate::link::LinkOptions;
use crate::record::{Record, Transfer};
use crate::{
    BulkRead, BulkWrite, DeviceSummary, EndpointDirection, InterfaceSummary, OpenOptions, Timeouts,
    UsbError, UsbIo,
};

/// Bytes in one simulated bulk packet. A real high-speed bulk endpoint's
/// packet size; the link halves ask for it in every IN transfer.
pub const SIM_PACKET_BYTES: usize = 512;

/// Bytes a simulated link buffers in each direction before the sender must
/// wait. A device's endpoint buffer is bounded, so the sender waits for the
/// other side to read; this bound is what keeps memory bounded in the tests.
pub const SIM_BUFFER_BYTES: usize = 8 * 1024;

/// How a simulated device answers the requests for its interface strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StringRequests {
    /// Answers with the strings it was given.
    #[default]
    Answer,
    /// Stalls the request, as a device with a broken descriptor does.
    Stall,
    /// Never answers: the request times out after the control timeout.
    Silent,
}

/// One scripted answer to one transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The transfer completes with no data.
    Ok,
    /// A read returns these bytes, cut to the length asked for. Fewer bytes
    /// is a short read. For a write this completes it.
    Data(Vec<u8>),
    /// The endpoint stalls.
    Stall,
    /// No answer arrives: the transfer fails after its timeout.
    Timeout,
    /// A write takes only this many bytes. Fewer than sent is an error.
    ShortWrite(usize),
    /// The device answers with something the protocol does not allow.
    Protocol(String),
    /// The device resets: it leaves the bus at this transfer.
    Disconnect,
}

#[derive(Debug)]
struct Slot {
    summary: DeviceSummary,
    generation: u64,
    answers: VecDeque<Answer>,
    records: Vec<Record>,
    interfaces: Vec<InterfaceSummary>,
    string_requests: StringRequests,
    claimed: Vec<u8>,
    pipe: Arc<Pipe>,
}

#[derive(Debug, Default)]
struct State {
    slots: HashMap<String, Slot>,
    generations: u64,
    subscribers: Vec<mpsc::UnboundedSender<HotplugEvent>>,
}

impl State {
    fn announce(&mut self, event: HotplugEvent) {
        // A subscriber whose stream was dropped is forgotten here.
        self.subscribers.retain(|tx| tx.send(event.clone()).is_ok());
    }

    fn remove(&mut self, location: &str) -> bool {
        if let Some(slot) = self.slots.remove(location) {
            // Handles on the old enumeration, and the peer, see the unplug.
            slot.pipe.close();
            self.announce(HotplugEvent::Detached {
                location: location.to_string(),
            });
            true
        } else {
            false
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A simulated bus. Clones share the same devices.
#[derive(Clone, Debug)]
pub struct SimBus {
    state: Arc<Mutex<State>>,
    gauges: Gauges,
}

impl SimBus {
    /// An empty bus with its own counters.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            gauges: Gauges::new(),
        }
    }

    /// The counters for this bus's device handles and watches.
    pub fn gauges(&self) -> Gauges {
        self.gauges.clone()
    }

    /// Plugs a device in at its location. Attaching at a location that is
    /// already taken re-enumerates it: the old device and its handles go.
    pub fn attach(&self, summary: DeviceSummary) {
        let mut state = lock(&self.state);
        let location = summary.location.clone();
        state.remove(&location);
        state.generations += 1;
        let generation = state.generations;
        state.slots.insert(
            location,
            Slot {
                summary: summary.clone(),
                generation,
                answers: VecDeque::new(),
                records: Vec::new(),
                interfaces: Vec::new(),
                string_requests: StringRequests::Answer,
                claimed: Vec::new(),
                pipe: Arc::new(Pipe::new()),
            },
        );
        state.announce(HotplugEvent::Attached(summary));
    }

    /// Unplugs the device at `location`. `false` if none was there.
    pub fn detach(&self, location: &str) -> bool {
        lock(&self.state).remove(location)
    }

    /// Queues answers for the device at `location`, used one per transfer.
    /// `false` if no device is there.
    pub fn script(&self, location: &str, answers: impl IntoIterator<Item = Answer>) -> bool {
        match lock(&self.state).slots.get_mut(location) {
            Some(slot) => {
                slot.answers.extend(answers);
                true
            }
            None => false,
        }
    }

    /// Gives the device at `location` its interfaces. A device with none
    /// opens as before, with no interface to pick. `false` if no device is
    /// there.
    pub fn set_interfaces(
        &self,
        location: &str,
        interfaces: impl IntoIterator<Item = InterfaceSummary>,
    ) -> bool {
        match lock(&self.state).slots.get_mut(location) {
            Some(slot) => {
                slot.interfaces = interfaces.into_iter().collect();
                true
            }
            None => false,
        }
    }

    /// Sets how the device at `location` answers its interface string
    /// requests. `false` if no device is there.
    pub fn set_string_requests(&self, location: &str, requests: StringRequests) -> bool {
        match lock(&self.state).slots.get_mut(location) {
            Some(slot) => {
                slot.string_requests = requests;
                true
            }
            None => false,
        }
    }

    /// The interface numbers opened at `location` since it was attached, in
    /// order. Empty if none is there or it has no interfaces.
    pub fn claimed(&self, location: &str) -> Vec<u8> {
        lock(&self.state)
            .slots
            .get(location)
            .map(|slot| slot.claimed.clone())
            .unwrap_or_default()
    }

    /// Every transfer the device at `location` has answered since it was
    /// attached, in order. Empty if none is there.
    pub fn records(&self, location: &str) -> Vec<Record> {
        lock(&self.state)
            .slots
            .get(location)
            .map(|slot| slot.records.clone())
            .unwrap_or_default()
    }

    /// The peer end of the bulk link on the device at `location`: what the
    /// board side sends and receives. `None` if no device is there.
    pub fn peer(&self, location: &str) -> Option<SimPeer> {
        lock(&self.state).slots.get(location).map(|slot| SimPeer {
            pipe: Arc::clone(&slot.pipe),
        })
    }

    /// Hotplug events from now on.
    pub fn subscribe(&self) -> SimEvents {
        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.state).subscribers.push(tx);
        SimEvents { rx }
    }

    /// A weak handle to the bus's state, for tests that check it is freed.
    #[cfg(test)]
    pub(crate) fn probe(&self) -> std::sync::Weak<dyn std::any::Any + Send + Sync> {
        let state: std::sync::Weak<Mutex<State>> = Arc::downgrade(&self.state);
        state
    }
}

impl Default for SimBus {
    fn default() -> Self {
        Self::new()
    }
}

/// The hotplug events of a [`SimBus`].
#[derive(Debug)]
pub struct SimEvents {
    rx: mpsc::UnboundedReceiver<HotplugEvent>,
}

impl Stream for SimEvents {
    type Item = HotplugEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<HotplugEvent>> {
        self.rx.poll_recv(cx)
    }
}

#[async_trait]
impl UsbBus for SimBus {
    async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
        let mut devices: Vec<DeviceSummary> = lock(&self.state)
            .slots
            .values()
            .map(|slot| slot.summary.clone())
            .collect();
        devices.sort_by(|a, b| a.location.cmp(&b.location));
        Ok(devices)
    }

    async fn interfaces(
        &self,
        location: &str,
        timeouts: Timeouts,
    ) -> Result<Vec<InterfaceSummary>, UsbError> {
        let (interfaces, requests) = {
            let state = lock(&self.state);
            let slot = state
                .slots
                .get(location)
                .ok_or_else(|| UsbError::NotFound(location.to_string()))?;
            (slot.interfaces.clone(), slot.string_requests)
        };
        // A device with no interface strings is never asked for one.
        let asks = interfaces
            .iter()
            .any(|interface| interface.string.is_some());
        match requests {
            StringRequests::Answer => Ok(interfaces),
            StringRequests::Stall if asks => Err(UsbError::Stall),
            StringRequests::Silent if asks => {
                tokio::time::sleep(timeouts.control).await;
                Err(UsbError::Timeout("the interface string request".into()))
            }
            StringRequests::Stall | StringRequests::Silent => Ok(interfaces),
        }
    }

    async fn open(
        &self,
        location: &str,
        options: &OpenOptions,
    ) -> Result<Box<dyn UsbIo>, UsbError> {
        let (generation, pipe) = {
            let mut state = lock(&self.state);
            let slot = state
                .slots
                .get_mut(location)
                .ok_or_else(|| UsbError::NotFound(location.to_string()))?;
            if !slot.interfaces.is_empty() {
                // Claim the named interface, or else the first one with a
                // bulk endpoint, as the real bus does.
                let chosen = match options.interface {
                    Some(number) => slot.interfaces.iter().find(|i| i.number == number),
                    None => slot.interfaces.iter().find(|i| has_bulk(i)),
                }
                .filter(|interface| has_bulk(interface))
                .ok_or_else(|| {
                    UsbError::Protocol(format!("no bulk endpoint to claim at {location}"))
                })?;
                let number = chosen.number;
                slot.claimed.push(number);
            }
            (slot.generation, Arc::clone(&slot.pipe))
        };
        let io: Box<dyn UsbIo> = Box::new(SimIo {
            state: Arc::clone(&self.state),
            location: location.to_string(),
            generation,
            timeouts: options.timeouts,
            pipe,
            _tracked: Tracked::new(&self.gauges, Kind::Device),
        });
        Ok(io)
    }
}

fn has_bulk(interface: &InterfaceSummary) -> bool {
    interface
        .endpoints
        .iter()
        .any(|ep| ep.kind == crate::EndpointKind::Bulk)
}

/// What a transfer resolved to, before any waiting.
enum Outcome {
    Now(Result<Vec<u8>, UsbError>),
    /// Wait for the timeout, then fail.
    Late,
}

/// One open handle on a simulated device.
pub struct SimIo {
    state: Arc<Mutex<State>>,
    location: String,
    generation: u64,
    timeouts: Timeouts,
    pipe: Arc<Pipe>,
    _tracked: Tracked,
}

impl std::fmt::Debug for SimIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimIo")
            .field("location", &self.location)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl SimIo {
    async fn run(&mut self, transfer: Transfer) -> Result<Vec<u8>, UsbError> {
        let timeout = match transfer {
            Transfer::ControlOut { .. } | Transfer::ControlIn { .. } => self.timeouts.control,
            Transfer::BulkOut(_) | Transfer::BulkIn { .. } => self.timeouts.bulk,
        };
        let (reply, wait) = {
            let mut state = lock(&self.state);
            let current = state
                .slots
                .get(&self.location)
                .is_some_and(|slot| slot.generation == self.generation);
            if !current {
                return Err(UsbError::Disconnected);
            }
            let answer = state
                .slots
                .get_mut(&self.location)
                .and_then(|slot| slot.answers.pop_front());
            let outcome = match answer {
                None => Outcome::Now(Err(UsbError::Protocol(format!(
                    "the simulated device has no scripted answer for: {}",
                    transfer.describe()
                )))),
                Some(Answer::Ok) => Outcome::Now(Ok(Vec::new())),
                Some(Answer::Data(data)) => Outcome::Now(Ok(fit(data, &transfer))),
                Some(Answer::Stall) => Outcome::Now(Err(UsbError::Stall)),
                Some(Answer::Timeout) => Outcome::Late,
                Some(Answer::ShortWrite(took)) => Outcome::Now(short_write(took, &transfer)),
                Some(Answer::Protocol(message)) => Outcome::Now(Err(UsbError::Protocol(message))),
                Some(Answer::Disconnect) => {
                    state.remove(&self.location);
                    Outcome::Now(Err(UsbError::Disconnected))
                }
            };
            let (reply, wait) = match outcome {
                Outcome::Now(reply) => (reply, None),
                Outcome::Late => (
                    Err(UsbError::Timeout(transfer.what().to_string())),
                    Some(timeout),
                ),
            };
            if let Some(slot) = state.slots.get_mut(&self.location) {
                slot.records.push(Record {
                    transfer,
                    reply: reply.clone(),
                });
            }
            (reply, wait)
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
        reply
    }
}

/// A read's bytes, cut to what was asked for. A write has no reply bytes.
fn fit(data: Vec<u8>, transfer: &Transfer) -> Vec<u8> {
    match transfer {
        Transfer::ControlIn { length, .. } => data[..data.len().min(usize::from(*length))].to_vec(),
        Transfer::BulkIn { max } => data[..data.len().min(*max)].to_vec(),
        Transfer::ControlOut { .. } | Transfer::BulkOut(_) => Vec::new(),
    }
}

fn short_write(took: usize, transfer: &Transfer) -> Result<Vec<u8>, UsbError> {
    let sent = match transfer {
        Transfer::ControlOut { data, .. } | Transfer::BulkOut(data) => data.len(),
        Transfer::ControlIn { .. } | Transfer::BulkIn { .. } => {
            return Err(UsbError::Protocol("a short-write answer to a read".into()));
        }
    };
    if took < sent {
        Err(UsbError::Protocol(format!("took {took} of {sent} bytes")))
    } else {
        Ok(Vec::new())
    }
}

#[async_trait]
impl UsbIo for SimIo {
    fn into_halves(self: Box<Self>, options: &LinkOptions) -> Result<crate::Halves, UsbError> {
        let SimIo { pipe, _tracked, .. } = *self;
        let tracked = Arc::new(_tracked);
        let reader = SimReadHalf {
            pipe: Arc::clone(&pipe),
            queued: 0,
            timeout: options.read_timeout,
            _tracked: Arc::clone(&tracked),
        };
        let writer = SimWriteHalf {
            pipe,
            queue: VecDeque::new(),
            timeout: options.write_timeout,
            _tracked: tracked,
        };
        Ok((Box::new(reader), Box::new(writer)))
    }

    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        self.run(Transfer::ControlOut {
            request,
            value,
            index,
            data: data.to_vec(),
        })
        .await
        .map(|_| ())
    }

    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        self.run(Transfer::ControlIn {
            request,
            value,
            index,
            length,
        })
        .await
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        self.run(Transfer::BulkOut(data.to_vec())).await.map(|_| ())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        self.run(Transfer::BulkIn { max }).await
    }
}

/// The state of one simulated bulk link. `to_pc` holds what the peer sent,
/// as USB packets, and `to_board` what the PC wrote. A device that is
/// unplugged closes it: every waiter wakes and fails with the gone kind.
#[derive(Debug)]
pub(crate) struct Pipe {
    state: Mutex<PipeState>,
    changed: Notify,
}

#[derive(Debug, Default)]
struct PipeState {
    gone: bool,
    stall_in: bool,
    stall_out: bool,
    to_pc: VecDeque<Vec<u8>>,
    to_pc_bytes: usize,
    to_board: VecDeque<Vec<u8>>,
    to_board_bytes: usize,
}

impl Pipe {
    fn new() -> Self {
        Self {
            state: Mutex::new(PipeState::default()),
            changed: Notify::new(),
        }
    }

    fn close(&self) {
        lock_pipe(self).gone = true;
        self.changed.notify_waiters();
    }
}

fn lock_pipe(pipe: &Pipe) -> MutexGuard<'_, PipeState> {
    pipe.state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Waits until `ready` returns `Some`. It sees the unplug, and any stall, as
/// errors. Cancel-safe: `ready` only looks, and changes nothing until it
/// returns `Some`.
async fn wait_until<T>(
    pipe: &Pipe,
    mut ready: impl FnMut(&mut PipeState) -> Result<Option<T>, UsbError>,
) -> Result<T, UsbError> {
    loop {
        // Made before the check, so a change between the check and the wait
        // is not missed.
        let changed = pipe.changed.notified();
        {
            let mut state = lock_pipe(pipe);
            if state.gone {
                return Err(UsbError::Disconnected);
            }
            if let Some(value) = ready(&mut state)? {
                return Ok(value);
            }
        }
        changed.await;
    }
}

/// How many queued packets make one IN transfer that asks for `max` bytes,
/// if they are all there: a short packet ends a transfer, and so does `max`.
fn read_ready(state: &PipeState, max: usize) -> Option<usize> {
    let mut total = 0;
    for (count, packet) in state.to_pc.iter().enumerate() {
        total += packet.len();
        if packet.len() < SIM_PACKET_BYTES || total >= max {
            return Some(count + 1);
        }
    }
    None
}

/// The read half of a simulated link.
#[derive(Debug)]
pub(crate) struct SimReadHalf {
    pipe: Arc<Pipe>,
    queued: usize,
    timeout: Option<Duration>,
    _tracked: Arc<Tracked>,
}

#[async_trait]
impl BulkRead for SimReadHalf {
    fn max_packet(&self) -> usize {
        SIM_PACKET_BYTES
    }

    fn submit(&mut self) {
        self.queued += 1;
    }

    fn pending(&self) -> usize {
        self.queued
    }

    async fn cancel(&mut self) {
        self.queued = 0;
    }

    async fn complete(&mut self) -> Result<Vec<u8>, UsbError> {
        let pipe = Arc::clone(&self.pipe);
        let wait = wait_until(&pipe, |state| {
            if state.stall_in {
                return Err(UsbError::Stall);
            }
            Ok(read_ready(state, SIM_PACKET_BYTES).map(|count| {
                let mut bytes = Vec::new();
                for packet in state.to_pc.drain(..count) {
                    state.to_pc_bytes -= packet.len();
                    bytes.extend(packet);
                }
                bytes
            }))
        });
        let outcome = match self.timeout {
            None => wait.await,
            Some(limit) => match tokio::time::timeout(limit, wait).await {
                Ok(outcome) => outcome,
                Err(_) => {
                    // Nothing is taken until a transfer completes, so a
                    // timeout loses no bytes.
                    self.queued = 0;
                    return Err(UsbError::Timeout("a bulk read".into()));
                }
            },
        };
        self.queued = self.queued.saturating_sub(1);
        if outcome.is_ok() {
            self.pipe.changed.notify_waiters();
        }
        outcome
    }
}

/// The write half of a simulated link. A queued transfer reaches the board's
/// side when there is room for it in [`SIM_BUFFER_BYTES`].
#[derive(Debug)]
pub(crate) struct SimWriteHalf {
    pipe: Arc<Pipe>,
    queue: VecDeque<Vec<u8>>,
    timeout: Option<Duration>,
    _tracked: Arc<Tracked>,
}

#[async_trait]
impl BulkWrite for SimWriteHalf {
    fn max_packet(&self) -> usize {
        SIM_PACKET_BYTES
    }

    fn submit(&mut self, data: Vec<u8>) {
        self.queue.push_back(data);
    }

    fn pending(&self) -> usize {
        self.queue.len()
    }

    async fn complete(&mut self) -> Result<(), UsbError> {
        let Some(front) = self.queue.front() else {
            return Err(UsbError::Protocol("no OUT transfer is queued".into()));
        };
        let len = front.len();
        let pipe = Arc::clone(&self.pipe);
        let wait = wait_until(&pipe, |state| {
            if state.stall_out {
                return Err(UsbError::Stall);
            }
            if state.to_board_bytes >= SIM_BUFFER_BYTES {
                return Ok(None);
            }
            Ok(Some(()))
        });
        let outcome = match self.timeout {
            None => wait.await,
            Some(limit) => match tokio::time::timeout(limit, wait).await {
                Ok(outcome) => outcome,
                Err(_) => {
                    self.queue.clear();
                    return Err(UsbError::Timeout("a bulk write".into()));
                }
            },
        };
        let Some(data) = self.queue.pop_front() else {
            return Err(UsbError::Protocol("no OUT transfer is queued".into()));
        };
        outcome?;
        {
            let mut state = lock_pipe(&pipe);
            if data.is_empty() {
                state.to_board.push_back(Vec::new());
            }
            for packet in data.chunks(SIM_PACKET_BYTES) {
                state.to_board.push_back(packet.to_vec());
            }
            state.to_board_bytes += len;
        }
        pipe.changed.notify_waiters();
        Ok(())
    }
}

/// The board's end of a simulated link. The test holds it while the PC side
/// runs: it sends packets the PC reads, and reads the packets the PC wrote,
/// with their boundaries as they were on the bus.
#[derive(Clone, Debug)]
pub struct SimPeer {
    pipe: Arc<Pipe>,
}

impl SimPeer {
    /// Sends one packet to the PC. At most [`SIM_PACKET_BYTES`]; a shorter
    /// packet ends the IN transfer that carries it, and an empty one is a
    /// zero-length packet. Waits while [`SIM_BUFFER_BYTES`] of packets wait
    /// unread.
    pub async fn send_packet(&self, bytes: &[u8]) -> Result<(), UsbError> {
        if bytes.len() > SIM_PACKET_BYTES {
            return Err(UsbError::Protocol(format!(
                "a packet is at most {SIM_PACKET_BYTES} bytes, got {}",
                bytes.len()
            )));
        }
        wait_until(&self.pipe, |state| {
            Ok((state.to_pc_bytes < SIM_BUFFER_BYTES).then_some(()))
        })
        .await?;
        let mut state = lock_pipe(&self.pipe);
        state.to_pc_bytes += bytes.len();
        state.to_pc.push_back(bytes.to_vec());
        drop(state);
        self.pipe.changed.notify_waiters();
        Ok(())
    }

    /// The next packet the PC wrote, exactly as it was sent. `None` once the
    /// device is gone.
    pub async fn recv_packet(&self) -> Option<Vec<u8>> {
        let packet = wait_until(&self.pipe, |state| {
            Ok(state.to_board.pop_front().inspect(|packet| {
                state.to_board_bytes -= packet.len();
            }))
        })
        .await
        .ok();
        self.pipe.changed.notify_waiters();
        packet
    }

    /// Bytes the PC has written that the peer has not received yet.
    pub fn queued_from_pc(&self) -> usize {
        lock_pipe(&self.pipe).to_board_bytes
    }

    /// Makes the endpoint in `direction` stall: `In` for the PC's reads, `Out`
    /// for its writes, until cleared with `false`.
    pub fn set_stall(&self, direction: EndpointDirection, stalled: bool) {
        let mut state = lock_pipe(&self.pipe);
        match direction {
            EndpointDirection::In => state.stall_in = stalled,
            EndpointDirection::Out => state.stall_out = stalled,
        }
        drop(state);
        self.pipe.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn device(location: &str) -> DeviceSummary {
        DeviceSummary::new(location, 0x0a5c, 0x2764).with_serial("abc")
    }

    #[tokio::test]
    async fn a_simulated_device_reports_the_versions_it_was_attached_with() {
        let bus = SimBus::new();
        let mut attached = device("1-1");
        attached.bcd_usb = 0x0201;
        attached.bcd_device = 0x0304;
        bus.attach(attached.clone());

        let listed = bus.devices().await.unwrap();
        assert_eq!(listed, vec![attached]);
        assert_eq!(listed[0].bcd_usb, 0x0201);
        assert_eq!(listed[0].bcd_device, 0x0304);
    }

    #[tokio::test]
    async fn stalls_short_reads_and_long_replies_are_as_scripted() {
        let bus = SimBus::new();
        bus.attach(device("1-1"));
        bus.script(
            "1-1",
            [
                Answer::Stall,
                Answer::Data(vec![1, 2, 3, 4]),
                Answer::Data(vec![5, 6, 7, 8, 9]),
            ],
        );
        let mut io = bus.open("1-1", &OpenOptions::default()).await.unwrap();
        assert_eq!(io.bulk_in(8).await, Err(UsbError::Stall));
        // Fewer bytes than asked is a normal short read.
        assert_eq!(io.bulk_in(8).await.unwrap(), vec![1, 2, 3, 4]);
        // More than asked is cut to the asked length.
        assert_eq!(io.bulk_in(2).await.unwrap(), vec![5, 6]);
    }

    #[tokio::test]
    async fn short_writes_report_how_much_was_taken() {
        let bus = SimBus::new();
        bus.attach(device("1-1"));
        bus.script("1-1", [Answer::ShortWrite(3), Answer::ShortWrite(8)]);
        let mut io = bus.open("1-1", &OpenOptions::default()).await.unwrap();
        assert_eq!(
            io.bulk_out(&[0; 8]).await,
            Err(UsbError::Protocol("took 3 of 8 bytes".into()))
        );
        assert_eq!(io.bulk_out(&[0; 8]).await, Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_timeout_takes_the_configured_time() {
        let bus = SimBus::new();
        bus.attach(device("1-1"));
        bus.script("1-1", [Answer::Timeout]);
        let options = OpenOptions {
            interface: None,
            timeouts: Timeouts {
                control: Duration::from_secs(1),
                bulk: Duration::from_secs(3),
            },
        };
        let mut io = bus.open("1-1", &options).await.unwrap();
        let started = tokio::time::Instant::now();
        assert_eq!(
            io.bulk_in(4).await,
            Err(UsbError::Timeout("a bulk read".into()))
        );
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test]
    async fn a_disconnect_kills_old_handles_until_the_device_is_opened_again() {
        let bus = SimBus::new();
        bus.attach(device("1-1"));
        let mut old = bus.open("1-1", &OpenOptions::default()).await.unwrap();
        bus.script("1-1", [Answer::Disconnect, Answer::Ok]);
        assert_eq!(old.bulk_out(&[1]).await, Err(UsbError::Disconnected));
        assert!(bus.devices().await.unwrap().is_empty());

        // The device resets and comes back at the same location: a new
        // enumeration. The old handle stays dead and does not use the queued
        // answer.
        bus.attach(device("1-1"));
        bus.script("1-1", [Answer::Ok]);
        assert_eq!(old.bulk_out(&[1]).await, Err(UsbError::Disconnected));
        let mut fresh = bus.open("1-1", &OpenOptions::default()).await.unwrap();
        assert_eq!(fresh.bulk_out(&[1]).await, Ok(()));
    }

    #[tokio::test]
    async fn a_transfer_without_an_answer_fails_loudly() {
        let bus = SimBus::new();
        bus.attach(device("1-1"));
        let mut io = bus.open("1-1", &OpenOptions::default()).await.unwrap();
        assert!(matches!(
            io.control_in(0x10, 0, 0, 4).await,
            Err(UsbError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn opening_a_missing_device_is_not_found() {
        let bus = SimBus::new();
        let error = bus.open("9-9", &OpenOptions::default()).await.err();
        assert_eq!(error, Some(UsbError::NotFound("9-9".into())));
    }

    #[tokio::test]
    async fn the_device_records_what_it_was_sent_and_answered() {
        let bus = SimBus::new();
        bus.attach(device("1-1"));
        bus.script("1-1", [Answer::Ok, Answer::Data(vec![7])]);
        let mut io = bus.open("1-1", &OpenOptions::default()).await.unwrap();
        io.control_out(0x01, 2, 3, &[9, 9]).await.unwrap();
        assert_eq!(io.bulk_in(4).await.unwrap(), vec![7]);
        let records = bus.records("1-1");
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].transfer,
            Transfer::ControlOut {
                request: 1,
                value: 2,
                index: 3,
                data: vec![9, 9]
            }
        );
        assert_eq!(records[1].reply, Ok(vec![7]));
    }
}
