//! Allwinner FEL: the boot ROM's USB download mode. Atlas loads a board from
//! its boot ROM with this crate, in Rust, with no vendor tool.
//!
//! The flow: verify the device is in FEL, write the SPL into SRAM through a
//! thunk (see [`thunk`]), read it back, run the thunk, check that FEL answers
//! again, then write an image to DRAM, read it back and run it.
//!
//! Built on the traits of `atlas-usb` ([`UsbIo`]) and nothing else of Atlas.
//! Files, USB ids and the SRAM layout come in from the caller as data.
//!
//! Clean-room, MIT. The code was written from public protocol notes (the
//! linux-sunxi wiki's FEL, FEL/Protocol, FEL/USBBoot and BROM pages, read from
//! web archive snapshots). No GPL source (sunxi-tools) and no vendor or U-Boot
//! FEL code was consulted.
//!
//! ## The thunk's contract (the ARMv7 thunk, `thunk.S`; the spec both
//! thunks satisfy)
//!
//! Entry: FEL_RUN jumps to the thunk's first byte in the boot ROM's execution
//! state, as a subroutine: the return address is in lr and returns into the
//! ROM's FEL loop. The thunk saves r4-r11, sp and lr in its own save area
//! (in the blob, outside every swap buffer), uses only r0-r3 and r12 and never
//! touches the stack, so the boot ROM's stack can be swapped out under it.
//!
//! Swap table: up to eight `(buf1, buf2, bytes)` triples of 32-bit words,
//! zero-terminated. Each swap exchanges `bytes` at `buf1` (a live FEL stack
//! region) with `bytes` at `buf2` (a free backup). The swap is its own
//! inverse.
//!
//! Pass 1: swap every table buffer. The SPL's bytes that would land on a live
//! buffer were written to its backup (see [`layout::Layout::spl_pieces`]), so
//! after this pass the SPL sits at its load address and the live stack sits
//! in the backups. Then the flag is set and the SPL entry word is called as a
//! subroutine (BLX; the SPL is entered with no arguments).
//!
//! Pass 2: the SPL returned, so swap every table buffer again. The flag is
//! set, so the thunk restores r4-r11, sp and lr from its save area and returns
//! into the ROM's FEL loop. The FEL loop then answers the next command.
//!
//! The thunk's code, table, entry word, flag and save area are all data the
//! host fills or checks: the offsets are in [`thunk`], and no address is a
//! constant of the code. The layout's rules keep the thunk outside every
//! swap and outside the SPL.
//!
//! ## The three modes (one [`layout::ThunkIsa`] each)
//!
//! - `Armv7` (`thunk.S`): FEL in AArch32, SPL in AArch32, returns to FEL.
//! - `Aarch64` (`thunk_a64.S`): FEL in native AArch64, the same contract with
//!   AAPCS64 callee-saved registers (x19-x30) and sp; returns to FEL; the SPL
//!   is called with BLR.
//! - `Aarch32ToAarch64` (`thunk_rmr.S`), for FEL in AArch32 with an AArch64
//!   payload. The thunk runs pass 1 only (no call, no return), writes the SPL's
//!   entry to the layout's reset vector register, and writes RMR with AA64 = 1
//!   and RR = 1, which requests a warm reset into AArch64. The core resets
//!   into the SPL: [`layout::ReturnMode::NoReturnExpected`]. FEL is not
//!   checked afterwards, no image can follow over FEL, and the device leaves
//!   the bus, which is the expected end. The flow after it (the SPL's own
//!   AArch32-to-AArch64 hook, DRAM init, the next stage) is the next stage's
//!   job, not this crate's.
//!
//! ## What is known and what is UNCONFIRMED for the A733
//!
//! Known, from the pinned Radxa U-Boot tree that B19 names (its Kconfig, read
//! for addresses only; commit 9da4b3e8):
//! - the SPL's SRAM base is `0x47000` (`SUNXI_SRAM_ADDRESS`), its maximum size
//!   `0x49000` (`CONFIG_SPL_MAX_SIZE`, so the SPL ends by `0x90000`), and the
//!   libdram runtime copy address is `0x90000` (`SUNXI_LIBDRAM_BASE`);
//! - the RVBAR backing store is `0x08001004` (`SUNXI_RVBAR_ADDRESS`), described
//!   there as the writable MMIO store that sets the AArch64 entry point,
//!   because RVBAR itself is read-only;
//! - BL31 is at `0x48000000`, the SCP firmware slot at `0x48100000` (not
//!   loaded by anything in B19's chain), DRAM from `0x40000000`;
//! - the U-Boot build inserts ARM32 code at the start of the binary that
//!   switches to AArch64 with an RMR write, and says the SPL and U-Boot can
//!   each be entered in either state. So the A733 SPL is designed to be
//!   entered in AArch32, which is the reason the RMR mode exists. This is a
//!   statement about the SPL, not about the boot ROM's FEL state.
//! - Gaia's build of B19 compiles the U-Boot SPL with its aarch64 toolchain
//!   (the build log names it); that says nothing about the entry state.
//!
//! UNCONFIRMED (each answered by a board check, read-only FEL queries first):
//! - the execution state FEL runs in on the A733 (AArch32 or AArch64): a FEL
//!   `ver` response and the boot ROM's state, read by a register dump after
//!   the first FEL verify on the A7S;
//! - whether a write to `0x08001004` followed by RMR really moves the core
//!   into AArch64 at `0x47000` on this SoC (the RVBAR store is documented, the
//!   sequence is not run);
//! - where the boot ROM keeps its FEL stack and data in SRAM, so the swap
//!   buffers and the thunk's own free SRAM are known; the verify response's
//!   scratchpad field is the first hint;
//! - whether SRAM survives the warm reset, and whether the reset enters the
//!   SPL or the boot ROM's own reset path first;
//! - the thunk address `0x90000` (just above the SPL's largest size) is used
//!   only by the no-return mode; the returning modes need a place the SPL
//!   never writes, which is not known;
//! - the DRAM address the flasher image is written to (`0x41000000` in the
//!   record: below BL31 and the SPL's FIT address; not checked);
//! - whether the device disconnects or times out when RMR resets it.
//!
//! The FEL command format (protocol.rs) and the mock device follow the wire
//! notes the crate was written from; neither proves the A733's boot ROM.
//!
//! Every transfer is logged at `info` under [`LOG_TARGET`].

pub mod client;
pub mod flash;
pub mod layout;
pub mod protocol;
pub mod sim;
pub mod thunk;

pub use atlas_usb::{UsbError, UsbIo};

/// The tracing target of every FEL transfer log.
pub const LOG_TARGET: &str = "atlas_fel";

/// One line per command step: what was sent and what came back.
pub(crate) fn trace(step: &str, request: &str, response: &[u8], expected: &str) {
    tracing::info!(
        target: LOG_TARGET,
        "{step}: request {request}; response {}; expected {expected}",
        hex(response)
    );
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
