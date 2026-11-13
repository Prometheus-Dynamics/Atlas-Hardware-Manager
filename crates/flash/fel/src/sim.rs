//! A stateful mock FEL device: it speaks the wire format of
//! [`crate::protocol`], keeps SRAM and one DRAM window, and runs the thunk the
//! way `thunk.S` does (the same swap table, the SPL call, the swap back). It
//! implements [`UsbIo`], the trait every loader drives.
//!
//! [`atlas_usb::sim::SimBus`] scripts one answer per transfer, which is the
//! right tool for framing and faults (stalls, drops, short writes, bad
//! status). A FEL flash reads back what it wrote and runs what it loaded, so
//! its answers depend on earlier writes; this device keeps that state. Both
//! are used by the tests.
//!
//! The SoC id it reports is [`MOCK_SOC_ID`], not a real chip's id. It proves
//! the sequence, the framing and the swap semantics; it does not prove what a
//! real boot ROM or SPL does.

use std::collections::VecDeque;

use async_trait::async_trait;
use atlas_usb::{UsbError, UsbIo};

use crate::layout::{Layout, ThunkIsa};
use crate::protocol::{
    AW_USB_READ, AW_USB_WRITE, MODE_FEL, REQUEST_LEN, STATUS_LEN, VERIFY_LEN, cmd,
};
use crate::thunk::{self, TABLE_ENTRIES};

/// The SRAM the mock has: 0..0x60000.
pub const SRAM_LEN: usize = 0x6_0000;
/// The byte the mock fills each stack buffer with (the live stack).
pub const STACK_FILL: u8 = 0x5a;
/// The SoC id the mock reports (not a real chip's id).
pub const MOCK_SOC_ID: u32 = 0x0733_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    /// Nothing requested: zero bytes.
    Nothing,
    Verify,
    Status,
    Memory(u32),
}

enum Region {
    Sram(usize),
    Dram(usize),
}

/// A mock device. `dram_len` bytes of DRAM sit at the layout's image address.
#[derive(Debug)]
pub struct FelDevice {
    layout: Layout,
    spl_len: usize,
    sram: Vec<u8>,
    dram: Vec<u8>,
    mode: u16,
    /// A request block was read; its body (direction, length) is due.
    awaiting: Option<usize>,
    /// A DOWNLOAD message was read; its data (address, length) is due.
    download: Option<(u32, usize)>,
    read: Read,
    replies: VecDeque<Vec<u8>>,
    gone: bool,
    /// Every command the device was sent, in order.
    pub commands: Vec<u16>,
    /// The SRAM the SPL saw when the thunk called it (its first `spl_len`
    /// bytes). Empty until a thunk run.
    pub spl_seen: Vec<u8>,
    /// Each reset vector register write the thunk made: (register, value).
    pub reset_writes: Vec<(u32, u32)>,
    /// The entry the device warm-reset into, when an RMR thunk ran.
    pub warm_reset_into: Option<u32>,
}

impl FelDevice {
    /// A device for `layout`, with the SPL as `spl_len` bytes and `dram_len`
    /// bytes of DRAM at the image address. Each stack buffer is filled with
    /// [`STACK_FILL`], as a live FEL stack would be.
    pub fn new(layout: Layout, spl_len: usize, dram_len: usize) -> Self {
        let mut sram = vec![0u8; SRAM_LEN];
        for swap in &layout.swaps {
            let start = swap.buf1 as usize;
            sram[start..start + swap.size as usize].fill(STACK_FILL);
        }
        Self {
            layout,
            spl_len,
            sram,
            dram: vec![0u8; dram_len],
            mode: MODE_FEL,
            awaiting: None,
            download: None,
            read: Read::Nothing,
            replies: VecDeque::new(),
            gone: false,
            commands: Vec::new(),
            spl_seen: Vec::new(),
            reset_writes: Vec::new(),
            warm_reset_into: None,
        }
    }

    /// A device that reports `mode` instead of FEL, for a test that the
    /// flash refuses a device not in FEL.
    pub fn in_mode(mut self, mode: u16) -> Self {
        self.mode = mode;
        self
    }

    /// The SRAM bytes at `address..address+len`, for a test to inspect.
    pub fn sram(&self, address: u32, len: usize) -> &[u8] {
        &self.sram[address as usize..address as usize + len]
    }

    /// The DRAM bytes at `address..address+len`, for a test to inspect.
    pub fn dram(&self, address: u32, len: usize) -> &[u8] {
        let at = (address - self.layout.image_address) as usize;
        &self.dram[at..at + len]
    }

    fn span(&self, address: u32, len: usize) -> Result<Region, UsbError> {
        let start = address as usize;
        if start + len <= SRAM_LEN {
            return Ok(Region::Sram(start));
        }
        let base = self.layout.image_address as usize;
        if start >= base && start + len <= base + self.dram.len() {
            return Ok(Region::Dram(start - base));
        }
        Err(UsbError::Protocol(format!(
            "{len} bytes at {address:#010x} are outside SRAM and the DRAM window"
        )))
    }

    fn store(&mut self, address: u32, data: &[u8]) -> Result<(), UsbError> {
        match self.span(address, data.len())? {
            Region::Sram(at) => self.sram[at..at + data.len()].copy_from_slice(data),
            Region::Dram(at) => self.dram[at..at + data.len()].copy_from_slice(data),
        }
        Ok(())
    }

    fn load(&self, address: u32, len: usize) -> Result<Vec<u8>, UsbError> {
        Ok(match self.span(address, len)? {
            Region::Sram(at) => self.sram[at..at + len].to_vec(),
            Region::Dram(at) => self.dram[at..at + len].to_vec(),
        })
    }

    fn reply(&mut self, data: Vec<u8>, status: u8) {
        if !data.is_empty() {
            self.replies.push_back(data);
        }
        let mut csw = b"AWUS".to_vec();
        csw.extend_from_slice(&[0; 8]);
        csw.push(status);
        self.replies.push_back(csw);
    }

    /// The thunk's work, as its source does it. ARMv7 and AArch64: swap every
    /// table buffer, let the SPL see its load address, swap back, and return
    /// to FEL. RMR: swap, write the entry to the reset vector register, and
    /// request the warm reset: the device leaves FEL and nothing returns.
    /// Returns whether the device is still in FEL afterwards.
    fn run_thunk(&mut self) -> bool {
        let thunk = self.layout.thunk_address as usize;
        let shape = thunk::shape(self.layout.thunk_isa);
        let table_at = thunk + shape.table;
        let swaps: Vec<(u32, u32, u32)> = self.sram[table_at..table_at + TABLE_ENTRIES * 12]
            .chunks(12)
            .map(|entry| {
                let word = |at: usize| {
                    u32::from_le_bytes([entry[at], entry[at + 1], entry[at + 2], entry[at + 3]])
                };
                (word(0), word(4), word(8))
            })
            .take_while(|(_, _, size)| *size != 0)
            .collect();
        let swap_all = |sram: &mut [u8]| {
            for (buf1, buf2, size) in &swaps {
                for offset in 0..*size {
                    sram.swap((buf1 + offset) as usize, (buf2 + offset) as usize);
                }
            }
        };
        swap_all(&mut self.sram);
        let entry_at = thunk + shape.entry;
        let entry = u32::from_le_bytes([
            self.sram[entry_at],
            self.sram[entry_at + 1],
            self.sram[entry_at + 2],
            self.sram[entry_at + 3],
        ]);
        self.spl_seen = self.sram[entry as usize..entry as usize + self.spl_len].to_vec();
        if let ThunkIsa::Aarch32ToAarch64 { reset_vector } = self.layout.thunk_isa {
            // The entry goes to the reset vector register, then RMR asks for
            // the warm reset. The swapped buffers stay swapped: nothing returns.
            self.reset_writes.push((reset_vector, entry));
            self.warm_reset_into = Some(entry);
            return false;
        }
        swap_all(&mut self.sram);
        true
    }

    fn command(&mut self, body: &[u8]) {
        let command = u16::from_le_bytes([body[0], body[1]]);
        self.commands.push(command);
        let address = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
        let len = u32::from_le_bytes([body[8], body[9], body[10], body[11]]) as usize;
        match command {
            cmd::VERIFY_DEVICE => {
                self.read = Read::Verify;
                self.reply(Vec::new(), 0);
            }
            cmd::UPLOAD => {
                self.read = Read::Memory(address);
                self.reply(Vec::new(), 0);
            }
            cmd::DOWNLOAD => {
                self.download = Some((address, len));
                self.reply(Vec::new(), 0);
            }
            cmd::RUN => {
                if address == self.layout.thunk_address {
                    if self.run_thunk() {
                        self.reply(Vec::new(), 0);
                    } else {
                        // The warm reset took the device out of FEL.
                        self.gone = true;
                    }
                } else {
                    // Any other code takes over: the device leaves FEL.
                    self.gone = true;
                }
            }
            _ => self.reply(Vec::new(), 1),
        }
    }

    fn verify_block(&self) -> Vec<u8> {
        let mut block = vec![0u8; VERIFY_LEN];
        block[0..8].copy_from_slice(b"AWUSBFEX");
        block[8..12].copy_from_slice(&MOCK_SOC_ID.to_le_bytes());
        block[12..16].copy_from_slice(&1u32.to_le_bytes());
        block[16..18].copy_from_slice(&self.mode.to_le_bytes());
        block[18] = 0x44;
        block[19] = 0x08;
        block[20..24].copy_from_slice(&0x7e00u32.to_le_bytes());
        block
    }
}

#[async_trait]
impl UsbIo for FelDevice {
    async fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
        Err(UsbError::Protocol(
            "a FEL device takes no vendor requests".into(),
        ))
    }

    async fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, UsbError> {
        Err(UsbError::Protocol(
            "a FEL device takes no vendor requests".into(),
        ))
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        if self.gone {
            return Err(UsbError::Disconnected);
        }
        if data.is_empty() {
            // A zero-length packet ends a data stage; nothing to do here.
            return Ok(());
        }
        if let Some(len) = self.awaiting.take() {
            if data.len() != len {
                return Err(UsbError::Protocol(format!(
                    "a body of {} bytes, the request said {len}",
                    data.len()
                )));
            }
            if let Some((address, due)) = self.download.take() {
                if due != len {
                    return Err(UsbError::Protocol(format!(
                        "data of {len} bytes, the DOWNLOAD said {due}"
                    )));
                }
                self.store(address, data)?;
                self.read = Read::Status;
                self.reply(Vec::new(), 0);
            } else {
                if data.len() < 16 {
                    return Err(UsbError::Protocol("a command body under 16 bytes".into()));
                }
                self.command(data);
            }
            return Ok(());
        }
        if data.len() != REQUEST_LEN || &data[0..4] != b"AWUC" {
            return Err(UsbError::Protocol(format!(
                "{} bytes that aren't a request block",
                data.len()
            )));
        }
        let direction = data[16];
        let len = u32::from_le_bytes([data[18], data[19], data[20], data[21]]) as usize;
        match direction {
            AW_USB_WRITE => self.awaiting = Some(len),
            AW_USB_READ => {
                let reply = match self.read {
                    Read::Verify => self.verify_block(),
                    Read::Status => {
                        let mut status = vec![0u8; STATUS_LEN];
                        status[0..2].copy_from_slice(&0xffffu16.to_le_bytes());
                        status
                    }
                    Read::Memory(address) => self.load(address, len)?,
                    Read::Nothing => vec![0u8; len],
                };
                if reply.len() != len {
                    return Err(UsbError::Protocol(format!(
                        "a read of {len} bytes, the device has {}",
                        reply.len()
                    )));
                }
                self.reply(reply, 0);
            }
            other => {
                return Err(UsbError::Protocol(format!(
                    "request direction {other:#04x}"
                )));
            }
        }
        Ok(())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        if self.gone {
            return Err(UsbError::Disconnected);
        }
        let Some(mut data) = self.replies.pop_front() else {
            return Err(UsbError::Protocol(
                "the host reads, the device has nothing to send".into(),
            ));
        };
        data.truncate(max);
        Ok(data)
    }
}
