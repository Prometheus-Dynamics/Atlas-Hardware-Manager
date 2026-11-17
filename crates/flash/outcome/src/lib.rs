//! The flashing outcome vocabulary: what a flashing step ended with, and what
//! was observed, as typed values. Std only, with no dependencies, so the job
//! engine and the drivers can name a flash result without linking a USB stack.
//!
//! `atlas-usb` produces these values and re-exports them; nothing here opens a
//! device.

mod id;
mod outcome;
mod port;

pub use id::UsbId;
pub use outcome::{Absent, Deaf, Expect, Outcome, Party, Reason, Refused, Stop, TimedOut};
pub use port::{MAX_TIERS, PortPath, PortPathError};
