//! The first-flash write over Orion (D6), for `atlas-flash-flow`'s [`Flasher`](atlas_flash_flow::Flasher) and
//! [`FirstFlash`](atlas_flash_flow::FirstFlash) seams.
//!
//! The board's side is Lemnos's (next da86d0b3, FLASH-BCM2712; `docs/slots.md`, "First flash"), and Orion's (next
//! 3f60576c: a flasher runs `orion-comm` and `orion-node` with `hello.state = flasher`):
//! - [`OrionFlasher`] takes the node [`Reach`] gives once its signed hello says `state = flasher` and its `serial`
//!   is the target's. Another board is refused; a node not yet the flasher is waited for.
//! - [`BridgeFlash`] is the write: the transport manifest pushed to `lemnos/<device>/<name>.json`, the image streamed
//!   to `lemnos/<device>/<name>` into `system.flash/1`'s write, the board's answer read as fields
//!   (`raw_bytes=<n> outcome=<code>`, [`wire::committed`]), then `verify` through the bridge's call on the device
//!   that carries `system.flash/1`, then `reboot` on the device that carries `system.reboot/1`. Every count is checked against the image.
//! - [`OrionBoard`] is the node's session as the write needs it: pushes as `atlas-files` jobs, calls through
//!   `atlas-actions`. [`FlasherBoard`] is the seam, so the write is tested without a node.
//!
//! - [`SerialReach`] is the way to the flasher: its board's own address on the USB network (serial-hash-v1, the
//!   board's data file built in), from the target's serial, so the PC needs no configuration.
//!
//! Nothing here names a family or a board. The calls go by contract; only the file sink's path names the flasher
//! board file's device id.

mod address;
mod bridge;
mod orion;
pub mod wire;

pub use address::{SCHEME, SCHEME_FILE, Scheme, SerialReach};
pub use bridge::{BridgeFlash, Bytes, CallError, FLASH_DEVICE, FlasherBoard, REBOOT, VERIFY};
pub use orion::{FLASHER_STATE, OrionBoard, OrionFlasher, Reach, SERIAL_FACT, is_flasher_of};
