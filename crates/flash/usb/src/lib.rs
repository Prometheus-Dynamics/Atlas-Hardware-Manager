//! One USB layer for every boot ROM loader: open and claim a device, control
//! and bulk transfers with timeouts, hotplug, and a traced mode that records
//! what a loader sent and what the device answered.
//!
//! Loaders see only the traits [`UsbBus`] and [`UsbIo`]. [`NusbBus`] is the
//! real bus (nusb, on Linux, Windows and macOS). [`sim::SimBus`] is a
//! scriptable device for tests: stalls, short transfers, timeouts, and
//! disconnects that re-enumerate mid-transfer, as a boot ROM does on reset.
//!
//! Nothing here knows which board or product a device is. A loader passes the
//! vendor and product ids it wants ([`UsbId`]) and matches devices itself.
//!
//! Timeouts, hotplug and [`await_device`] need a tokio runtime.

mod bus;
mod error;
mod gauge;
mod hotplug;
mod io;
mod nusb_backend;
mod record;
pub mod sim;
mod traced;

pub use bus::{UsbBus, await_device};
pub use error::UsbError;
pub use gauge::{Gauges, Live};
pub use hotplug::{HotplugEvent, Watch, spawn_watch, watch_usb};
pub use io::{DeviceSummary, OpenOptions, Timeouts, UsbId, UsbIo};
pub use nusb_backend::NusbBus;
pub use record::{Record, Transfer};
pub use traced::{Recorder, Sink, Traced};
