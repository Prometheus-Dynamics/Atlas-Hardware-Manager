//! The first-flash write over Orion (D6), for `atlas-flash-flow`'s [`Flasher`](atlas_flash_flow::Flasher) and
//! [`FirstFlash`](atlas_flash_flow::FirstFlash) seams.
//!
//! The board's side is Lemnos's (next 4b5be3a5, FLASH-BCM2712; `docs/slots.md`, "First flash"), and Orion's (next
//! 3f60576c: a flasher runs `orion-comm` and `orion-node` with `hello.state = flasher`):
//! - [`OrionFlasher`] takes the node [`Reach`] gives once its signed hello says `state = flasher` and its `serial`
//!   is the target's. Another board is refused; a node not yet the flasher is waited for.
//! - [`BridgeFlash`] is the write: the transport manifest pushed to `lemnos/<device>/<name>.json`, the image streamed
//!   to `lemnos/<device>/<name>` into `system.flash/1`'s write, the board's answer read as fields
//!   (`raw_bytes=<n> outcome=<code>`, [`wire::committed`]), then `verify` through the bridge's
//!   `lemnos.<device>.call`, then `reboot` on the restart device. Every count is checked against the image.
//! - [`OrionBoard`] is the node's session as the write needs it: pushes as `atlas-files` jobs, calls through
//!   `atlas-actions`. [`FlasherBoard`] is the seam, so the write is tested without a node.
//!
//! Nothing here names a family or a board; the device ids are the flasher board file's.

mod bridge;
mod orion;
pub mod wire;

pub use bridge::{BridgeFlash, Bytes, CallError, FLASH_DEVICE, FlasherBoard, REBOOT_DEVICE};
pub use orion::{FLASHER_STATE, OrionBoard, OrionFlasher, Reach, SERIAL_FACT, is_flasher_of};
