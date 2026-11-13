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
use tokio::sync::mpsc;

use crate::bus::UsbBus;
use crate::gauge::{Gauges, Kind, Tracked};
use crate::hotplug::HotplugEvent;
use crate::record::{Record, Transfer};
use crate::{DeviceSummary, OpenOptions, Timeouts, UsbError, UsbIo};

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
        if self.slots.remove(location).is_some() {
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

    /// Every transfer the device at `location` has answered since it was
    /// attached, in order. Empty if none is there.
    pub fn records(&self, location: &str) -> Vec<Record> {
        lock(&self.state)
            .slots
            .get(location)
            .map(|slot| slot.records.clone())
            .unwrap_or_default()
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

    async fn open(
        &self,
        location: &str,
        options: &OpenOptions,
    ) -> Result<Box<dyn UsbIo>, UsbError> {
        let generation = lock(&self.state)
            .slots
            .get(location)
            .map(|slot| slot.generation)
            .ok_or_else(|| UsbError::NotFound(location.to_string()))?;
        let io: Box<dyn UsbIo> = Box::new(SimIo {
            state: Arc::clone(&self.state),
            location: location.to_string(),
            generation,
            timeouts: options.timeouts,
            _tracked: Tracked::new(&self.gauges, Kind::Device),
        });
        Ok(io)
    }
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
