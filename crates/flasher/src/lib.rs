//! The raze-flasher daemon. [`session`] speaks the protocol over any byte
//! pipe; [`ffs`] makes that pipe the USB gadget's FunctionFS interface;
//! [`board`] reads what the board says about itself.

pub mod board;
#[cfg(target_os = "linux")]
pub mod ffs;
pub mod ring;
pub mod session;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
