//! Flashing SoCs from their USB download modes, in Rust, with no vendor
//! tool at run time: Rockchip maskrom and loader mode ([`rockchip`]) and the
//! Kendryte K230's boot ROM and burner ([`k230`]), over one small USB
//! interface ([`transport`]) that tests replay recorded traces through.
//!
//! Written from the vendors' host tools and docs; every protocol detail no
//! source confirmed is marked UNCONFIRMED where it's used. Untested on a
//! board: the boards are on order.

pub mod k230;
pub mod rockchip;
pub mod traced;
pub mod transport;

pub use traced::Traced;
pub use transport::{NusbIo, Script, Step, UsbError, UsbIo};

/// What a long transfer is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Writing,
    Verifying,
}

/// Hears (phase, bytes done, bytes in all).
pub type Progress<'a> = &'a mut (dyn FnMut(Phase, u64, u64) + Send);

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
