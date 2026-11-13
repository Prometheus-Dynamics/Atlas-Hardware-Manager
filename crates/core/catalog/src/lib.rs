//! The device catalog: how to recognise a board from what it shows (its USB
//! ids, its hostname), and what it looks like (its name, a picture, a case).
//!
//! # The app data
//!
//! The data lives under the repository's `devices/` folder, beside each
//! board's description (`devices/boards/<board>/`, the Gaia format). The
//! reader takes exactly two kinds of file and ignores everything else:
//!
//! - `devices/usb-ids.json`, the USB ids, shared by every board that shows
//!   them. A board names them by their key:
//!
//!   ```json
//!   {
//!     "schema": "urn:atlas:usb-ids:1",
//!     "ids": {
//!       "rockchip-rk3588-maskrom": { "state": "download", "vid": "2207", "pid": "350b", "name": "Rockchip RK3588 maskrom" },
//!       "rockchip-vendor": { "state": "download", "vid": "2207", "name": "Rockchip boot ROM, any product" }
//!     }
//!   }
//!   ```
//!
//!   `state` is `download`, `storage` or `gadget`. `vid` and `pid` are four
//!   hex digits. `pid` may be left out: the id is then vendor-wide, and
//!   matches any product of that vendor in that state. A board's exact
//!   `vid:pid` match always ranks above a vendor-wide one. A key is `[a-z0-9-]`.
//!
//! - `devices/boards/<board>/app/app.json`, one board's app data. Nothing
//!   else under a board is read (its Gaia files included). A board directory
//!   without `app/app.json` is not in the catalog.
//!
//!   ```json
//!   {
//!     "schema": "urn:atlas:device-app:1",
//!     "model": "example-board",
//!     "display_name": "Example Board",
//!     "usb": ["rockchip-rk3588-maskrom"],
//!     "network": { "hostname_prefix": "example-" },
//!     "look": {
//!       "picture": "picture.png",
//!       "pictures": ["side.png"],
//!       "case": "case.glb",
//!       "orientation": { "case_in_imu": [0, 0.7071, -0.7071, 0], "ring_led0_deg": 157.5, "ring_direction": -1 }
//!     },
//!     "ports": { "usb-c": { "kind": "usb-c", "label": "USB-C" } },
//!     "variants": [
//!       { "key": "wide", "display_name": "Wide lens", "confirmed": false, "overrides": { "case": "wide.glb" } }
//!     ],
//!     "found_by": "robot"
//!   }
//!   ```
//!
//!   The fields after `usb` are optional, and a file with only the first
//!   five is valid.
//!
//!   - `model` must equal the board's directory name (`example-board` above),
//!     and is `[a-z0-9-]`. `usb` lists keys from `usb-ids.json`.
//!   - `network.hostname_prefix` is `[a-z0-9-]`.
//!   - `look.picture` is the first picture; `look.pictures` lists more after
//!     it. `look.case` is the case model. All are files in the board's `app/`
//!     folder, which they must stay inside. The catalog stores them as paths
//!     relative to `devices/` (`boards/example-board/app/picture.png`) and
//!     does not read them. The same file twice is refused.
//!   - `look.orientation` says how the case sits: `case_in_imu` is a unit
//!     quaternion `[w, x, y, z]` taking case vectors to IMU vectors,
//!     `ring_led0_deg` is LED 0's angle in the case's frame (0 to 360, from
//!     +x toward +y), and `ring_direction` (1 or -1) is which way the rest of
//!     the ring runs. Each is optional.
//!   - `ports` is a map from port name to `{ kind, label }`. Names and kinds
//!     are `[a-z0-9-]`; the label is what the app shows.
//!   - `variants` are options of the same board, such as two lenses. Each has
//!     a `key` (`[a-z0-9-]`, unique on the board), a `display_name`, and
//!     `confirmed` (defaults to `true`; `false` marks facts not yet measured).
//!     Its `overrides` replace the board's `pictures`, `case` or `ports` when
//!     present. A variant does not change the board's `usb` or `orientation`.
//!   - `found_by` names the driver family that finds a device which shows no
//!     USB id of its own (for example a robot's controller), free text.
//!   - Unknown fields are an error everywhere.
//!
//! A missing `devices/` folder, or one with no such boards, is an empty
//! catalog.
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
pub use model::{Board, Catalog, Orientation, Overrides, Port, UsbId, UsbMatch, UsbState, Variant};

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
    /// Every board that has this USB id in this state. A board lists an exact
    /// `vid:pid`, or a vendor-wide id (no `pid`) that matches any product of
    /// the vendor. Exact matches come first; then the vendor-wide ones. Each
    /// group is in model order. One id can belong to more than one board (a
    /// shared boot ROM id): each is a candidate.
    pub fn recognise_usb(&self, vid: u16, pid: u16, state: UsbState) -> Vec<UsbMatch> {
        let mut exact = Vec::new();
        let mut vendor = Vec::new();
        for board in &self.boards {
            let on_board = |id: &UsbId| id.vid == vid && id.state == state;
            let is_exact = board
                .usb
                .iter()
                .any(|id| on_board(id) && id.pid == Some(pid));
            let is_vendor = board.usb.iter().any(|id| on_board(id) && id.pid.is_none());
            if !is_exact && !is_vendor {
                continue;
            }
            let found = UsbMatch {
                model: board.model.clone(),
                display_name: board.display_name.clone(),
                state,
                exact: is_exact,
            };
            if is_exact {
                exact.push(found);
            } else {
                vendor.push(found);
            }
        }
        exact.extend(vendor);
        exact
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
