//! The device catalog: how to recognise a board from what it shows (its USB
//! ids, its hostname), and what it looks like (its name, a picture, a case).
//!
//! # The app data
//!
//! The data lives under the repository's `devices/` folder, beside each
//! board's description. The reader takes only these two kinds of file and
//! ignores everything else in that folder, so the Gaia build files stay out
//! of the app.
//!
//! `devices/usb-ids.json` holds the USB ids, shared by every board that
//! shows them. A board names them by their key:
//!
//! ```json
//! {
//!   "schema": "urn:atlas:usb-ids:1",
//!   "ids": {
//!     "rockchip-rk3588-maskrom": { "state": "download", "vid": "2207", "pid": "350b", "name": "Rockchip RK3588 maskrom" },
//!     "rpi-storage-gadget": { "state": "storage", "vid": "0a5c", "pid": "0104", "name": "Raspberry Pi storage gadget" }
//!   }
//! }
//! ```
//!
//! `state` is `download`, `storage` or `gadget`. `vid` and `pid` are four hex
//! digits. The key is `[a-z0-9-]`.
//!
//! Each board has one `app.json` in its own folder, next to its description:
//!
//! ```json
//! {
//!   "schema": "urn:atlas:device-app:1",
//!   "model": "example-board",
//!   "display_name": "Example Board",
//!   "usb": ["rockchip-rk3588-maskrom"],
//!   "network": { "hostname_prefix": "example-" },
//!   "look": { "picture": "picture.png", "case": "case.glb" }
//! }
//! ```
//!
//! `model` is unique across the tree. `usb` lists keys from `usb-ids.json`.
//! `network` and `look` are optional. `picture` and `case` are files in the
//! board's own folder (they must stay inside it); the catalog stores them as
//! paths relative to `devices/`, and does not read them. Unknown fields are
//! an error.
//!
//! A missing `devices/` folder, or one with no app data, is an empty catalog.
//!
//! # The embedded catalog
//!
//! The build script reads `devices/` (from the repository this crate is in)
//! and writes the whole catalog as JSON into the build output. [`embedded`]
//! returns it. A bad file fails the build, so the app never embeds data it
//! could not read.

mod load;
mod model;

use std::path::Path;
use std::sync::OnceLock;

pub use load::{APP_FILE, APP_SCHEMA, CATALOG_SCHEMA, LoadError, USB_IDS_FILE, USB_IDS_SCHEMA};
pub use model::{Board, Catalog, UsbId, UsbMatch, UsbState};

/// The catalog the build script embedded from `devices/`.
pub fn embedded() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(EMBEDDED_JSON).expect("the build script wrote a valid catalog")
    })
}

/// The size of the embedded catalog, in bytes of JSON.
pub fn embedded_size() -> usize {
    EMBEDDED_JSON.len()
}

const EMBEDDED_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/catalog.json"));

/// Reads a `devices/` folder into a catalog. For tests and tools; the app uses
/// [`embedded`].
pub fn read_dir(devices: &Path) -> Result<Catalog, LoadError> {
    load::from_dir(devices)
}

impl Catalog {
    /// Every board that has this USB id in this state. One id can belong to
    /// more than one board (a shared boot ROM id): each is a candidate.
    pub fn recognise_usb(&self, vid: u16, pid: u16, state: UsbState) -> Vec<UsbMatch> {
        self.boards
            .iter()
            .filter(|board| {
                board
                    .usb
                    .iter()
                    .any(|id| id.vid == vid && id.pid == pid && id.state == state)
            })
            .map(|board| UsbMatch {
                model: board.model.clone(),
                display_name: board.display_name.clone(),
                state,
            })
            .collect()
    }

    /// Every board whose hostname prefix the hostname starts with. Case does
    /// not matter.
    pub fn recognise_hostname(&self, hostname: &str) -> Vec<&Board> {
        let host = hostname.to_ascii_lowercase();
        self.boards
            .iter()
            .filter(|board| {
                board
                    .hostname_prefix
                    .as_deref()
                    .is_some_and(|prefix| host.starts_with(prefix))
            })
            .collect()
    }

    /// The board with this model id.
    pub fn board(&self, model: &str) -> Option<&Board> {
        self.boards.iter().find(|board| board.model == model)
    }
}
