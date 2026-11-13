//! A simulated Rockchip boot ROM on [`atlas_usb::sim::SimBus`], for tests
//! and for the driver's tests. It takes a maskrom download (the DDR stage,
//! then the usbplug), re-enumerates at the same location in loader mode when
//! the usbplug's last chunk lands, and then answers the read-only rockusb
//! queries as rkdeveloptool's CBW/CSW framing expects.
//!
//! This is the dev simulator, rebuilt on the shared USB layer: the bus,
//! the device table and hotplug events are [`SimBus`]'s, and the transfers are
//! this board's. The board checks each stage's framing (see
//! [`crate::maskrom::stage_is_intact`]). It is only as right as the source
//! it was written from; it shows the sequence and the framing, not what a
//! real boot ROM answers. Tests use [`Fault`] to make it misbehave on purpose.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use atlas_usb::sim::SimBus;
use atlas_usb::{DeviceSummary, OpenOptions, Timeouts, UsbBus, UsbError, UsbId, UsbIo};

use crate::maskrom::{CHUNK, REQUEST, STAGE_DDR, STAGE_USBPLUG, stage_is_intact};
use crate::rockusb::op;

const CBW_LEN: usize = 31;
/// READ_FLASH_INFO's reply: the size, block and page fields, as the flash
/// layout's fields read in rkdeveloptool's struct.
const FLASH_INFO_LEN: usize = 512;

/// One way for the next transfer to go wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The endpoint stalls: the transfer fails at once.
    Stall,
    /// No answer: the transfer fails after its timeout.
    Timeout,
    /// The board drops off the bus. Handles opened before stay dead.
    Disconnect,
    /// One byte of the transfer's data is flipped on its way to the board, so
    /// the stage's CRC no longer matches. Only a maskrom data transfer has
    /// data to flip; the fault waits for the next one that does.
    CorruptChunk,
}

#[derive(Debug)]
struct State {
    bus: SimBus,
    summary: DeviceSummary,
    chip: String,
    sectors: u32,
    /// Storage the usbplug uses, as READ_STORAGE's mask bit (1 eMMC).
    storage: u32,
    generation: u64,
    present: bool,
    loader: bool,
    ddr_done: bool,
    /// The stage being received: its wIndex and its bytes so far.
    stage: Option<(u16, Vec<u8>)>,
    /// Completed stages, in order, by wIndex.
    stages: Vec<u16>,
    faults: VecDeque<Fault>,
    /// Replies queued for the host's next bulk IN: data, then a CSW.
    replies: VecDeque<Vec<u8>>,
}

impl State {
    fn alive(&self, generation: u64) -> bool {
        self.present && self.generation == generation
    }

    /// Drops the device off the bus: its handles die, and the bus forgets it.
    fn unplug(&mut self) {
        self.present = false;
        self.bus.detach(&self.summary.location);
    }

    /// The reset after the usbplug: the same location, now in loader mode,
    /// as a new enumeration.
    fn reenumerate(&mut self) {
        self.generation += 1;
        self.loader = true;
        self.stage = None;
        self.bus.attach(self.summary.clone());
    }

    fn csw(tag: u32, status: u8) -> Vec<u8> {
        let mut csw = b"USBS".to_vec();
        csw.extend_from_slice(&tag.to_le_bytes());
        csw.extend_from_slice(&[0; 4]);
        csw.push(status);
        csw
    }

    fn flash_info(&self) -> Vec<u8> {
        let mut info = Vec::with_capacity(FLASH_INFO_LEN);
        info.extend_from_slice(&self.sectors.to_le_bytes());
        info.extend_from_slice(&32u16.to_le_bytes());
        info.push(4);
        info.resize(FLASH_INFO_LEN, 0);
        info
    }

    fn chip_info(&self) -> Vec<u8> {
        let mut info: Vec<u8> = self.chip.bytes().rev().collect();
        info.resize(16, 0);
        info
    }

    /// Answers one CBW: its data (if the command has any) and then its CSW.
    fn command(&mut self, cbw: &[u8]) {
        let tag = u32::from_le_bytes([cbw[4], cbw[5], cbw[6], cbw[7]]);
        let transfer = u32::from_le_bytes([cbw[8], cbw[9], cbw[10], cbw[11]]) as usize;
        let reply = match cbw[15] {
            op::TEST_UNIT_READY => Some(Vec::new()),
            op::READ_FLASH_ID => Some(vec![0; transfer.min(5)]),
            op::READ_FLASH_INFO => Some(self.flash_info()[..transfer.min(FLASH_INFO_LEN)].to_vec()),
            op::READ_CHIP_INFO => Some(self.chip_info()[..transfer.min(16)].to_vec()),
            op::READ_CAPABILITY => Some(vec![0; transfer.min(8)]),
            op::READ_STORAGE => Some(self.storage.to_le_bytes().to_vec()),
            _ => None,
        };
        match reply {
            Some(data) => {
                if !data.is_empty() {
                    self.replies.push_back(data);
                }
                self.replies.push_back(Self::csw(tag, 0));
            }
            // Refused: a status of 1 and nothing else.
            None => self.replies.push_back(Self::csw(tag, 1)),
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A simulated Rockchip device at one location on a [`SimBus`] of its own.
#[derive(Clone, Debug)]
pub struct SimRockchip {
    state: Arc<Mutex<State>>,
}

impl SimRockchip {
    /// A device in maskrom at `location`, with the USB id `id`, reporting
    /// `chip` (the chip number, such as "3588") and `sectors` of storage.
    pub fn maskrom(location: &str, id: UsbId, chip: &str, sectors: u32) -> Self {
        Self::build(location, id, chip, sectors, false)
    }

    /// A device already in loader mode at `location`: its usbplug runs.
    pub fn loader(location: &str, id: UsbId, chip: &str, sectors: u32) -> Self {
        Self::build(location, id, chip, sectors, true)
    }

    fn build(location: &str, id: UsbId, chip: &str, sectors: u32, loader: bool) -> Self {
        let bus = SimBus::new();
        let summary = DeviceSummary {
            location: location.to_string(),
            vendor: id.vendor,
            product: id.product,
            serial: None,
        };
        bus.attach(summary.clone());
        Self {
            state: Arc::new(Mutex::new(State {
                bus,
                summary,
                chip: chip.to_string(),
                sectors,
                storage: 1 << 1,
                generation: 0,
                present: true,
                loader,
                ddr_done: false,
                stage: None,
                stages: Vec::new(),
                faults: VecDeque::new(),
                replies: VecDeque::new(),
            })),
        }
    }

    /// The bus the device is on. Hotplug subscriptions and the device table
    /// are the bus's; the board's transfers are this type's.
    pub fn bus(&self) -> SimBus {
        lock(&self.state).bus.clone()
    }

    /// Queues `fault` for the next transfer.
    pub fn inject(&self, fault: Fault) {
        lock(&self.state).faults.push_back(fault);
    }

    /// Unplugs the device.
    pub fn unplug(&self) {
        lock(&self.state).unplug();
    }

    /// Whether the usbplug is running (the device is in loader mode).
    pub fn in_loader_mode(&self) -> bool {
        lock(&self.state).loader
    }

    /// Completed maskrom stages, by wIndex, in the order they landed.
    pub fn stages(&self) -> Vec<u16> {
        lock(&self.state).stages.clone()
    }
}

#[async_trait]
impl UsbBus for SimRockchip {
    async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
        let bus = lock(&self.state).bus.clone();
        bus.devices().await
    }

    async fn open(
        &self,
        location: &str,
        options: &OpenOptions,
    ) -> Result<Box<dyn UsbIo>, UsbError> {
        let state = lock(&self.state);
        if !state.present || state.summary.location != location {
            return Err(UsbError::NotFound(location.to_string()));
        }
        Ok(Box::new(SimIo {
            state: Arc::clone(&self.state),
            generation: state.generation,
            timeouts: options.timeouts,
        }))
    }
}

/// One open handle on the simulated board.
struct SimIo {
    state: Arc<Mutex<State>>,
    generation: u64,
    timeouts: Timeouts,
}

impl SimIo {
    /// Checks the handle is live and takes the next fault, if any. A
    /// disconnect is applied here, under the lock. A [`Fault::CorruptChunk`]
    /// waits for a transfer that carries data, so it is only taken when
    /// `carries_data` is set.
    fn gate(&self, state: &mut State, carries_data: bool) -> Result<Option<Fault>, UsbError> {
        if !state.alive(self.generation) {
            return Err(UsbError::Disconnected);
        }
        match state.faults.front() {
            None => return Ok(None),
            Some(Fault::CorruptChunk) if !carries_data => return Ok(None),
            Some(_) => {}
        }
        match state.faults.pop_front() {
            Some(Fault::Disconnect) => {
                state.unplug();
                Err(UsbError::Disconnected)
            }
            other => Ok(other),
        }
    }
}

impl SimIo {
    /// A bulk transfer's fault: a stall fails it now, a timeout returns the
    /// wait the caller sleeps before failing it. Nothing is held across that
    /// wait.
    fn bulk_fault(&self, state: &mut State) -> Result<Option<Duration>, UsbError> {
        match self.gate(state, false)? {
            Some(Fault::Stall) => Err(UsbError::Stall),
            Some(Fault::Timeout) => Ok(Some(self.timeouts.bulk)),
            _ => Ok(None),
        }
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
        let mut data = data.to_vec();
        let timeout = {
            let mut state = lock(&self.state);
            match self.gate(&mut state, !data.is_empty())? {
                Some(Fault::Stall) => return Err(UsbError::Stall),
                Some(Fault::Timeout) => Some(self.timeouts.control),
                Some(Fault::CorruptChunk) => {
                    data[0] ^= 0x40;
                    None
                }
                _ => None,
            }
        };
        if let Some(wait) = timeout {
            tokio::time::sleep(wait).await;
            return Err(UsbError::Timeout("the maskrom download".into()));
        }

        let mut state = lock(&self.state);
        if state.loader {
            return Err(UsbError::Protocol(
                "a maskrom request after the usbplug ran".into(),
            ));
        }
        if request != REQUEST || value != 0 {
            return Err(UsbError::Protocol(format!(
                "request {request:#04x} value {value:#06x} isn't a maskrom download"
            )));
        }
        if index != STAGE_DDR && index != STAGE_USBPLUG {
            return Err(UsbError::Protocol(format!(
                "maskrom stage {index:#06x} isn't 0x0471 or 0x0472"
            )));
        }
        if data.len() > CHUNK {
            return Err(UsbError::Protocol(format!(
                "a {}-byte request; maskrom takes at most {CHUNK}",
                data.len()
            )));
        }
        if state.stage.as_ref().is_some_and(|(open, _)| *open != index) {
            state.stage = None;
            return Err(UsbError::Protocol(
                "a new maskrom stage before the last one ended".into(),
            ));
        }
        let mut bytes = state.stage.take().map_or_else(Vec::new, |(_, bytes)| bytes);
        bytes.extend_from_slice(&data);
        if data.len() == CHUNK {
            // A full chunk: the stage goes on.
            state.stage = Some((index, bytes));
            return Ok(());
        }
        // A short chunk ends the stage. Its framing decides whether it landed.
        if !stage_is_intact(&bytes) {
            return Err(UsbError::Protocol(format!(
                "maskrom stage {index:#06x}: its CRC doesn't match ({} bytes)",
                bytes.len()
            )));
        }
        state.stages.push(index);
        if index == STAGE_DDR {
            state.ddr_done = true;
            Ok(())
        } else if state.ddr_done {
            // The usbplug runs; the board resets and comes back in loader mode.
            state.reenumerate();
            Ok(())
        } else {
            Err(UsbError::Protocol(
                "the usbplug stage came before the DDR init".into(),
            ))
        }
    }

    async fn control_in(
        &mut self,
        request: u8,
        _value: u16,
        _index: u16,
        _length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        Err(UsbError::Protocol(format!(
            "the simulated board answers no vendor read (request {request:#04x})"
        )))
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        let wait = {
            let mut state = lock(&self.state);
            self.bulk_fault(&mut state)?
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
            return Err(UsbError::Timeout("a rockusb command".into()));
        }
        let mut state = lock(&self.state);
        if !state.loader {
            return Err(UsbError::Protocol(
                "a rockusb command before the usbplug ran".into(),
            ));
        }
        if data.len() != CBW_LEN || data[0..4] != *b"USBC" {
            return Err(UsbError::Protocol(format!(
                "{} bytes that aren't a CBW",
                data.len()
            )));
        }
        state.command(data);
        Ok(())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        let wait = {
            let mut state = lock(&self.state);
            self.bulk_fault(&mut state)?
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
            return Err(UsbError::Timeout("a rockusb reply".into()));
        }
        let mut state = lock(&self.state);
        let Some(mut reply) = state.replies.pop_front() else {
            return Err(UsbError::Protocol(
                "the host reads, but the board has nothing to send".into(),
            ));
        };
        reply.truncate(max);
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csw_is_thirteen_bytes_with_its_status() {
        let csw = State::csw(7, 1);
        assert_eq!(csw.len(), 13);
        assert_eq!(&csw[0..4], b"USBS");
        assert_eq!(csw[12], 1);
    }
}
