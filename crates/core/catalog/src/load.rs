//! Reads the app data under a `devices/` folder. Shared by the build script
//! (which embeds the result) and the library's tests.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::model::{Board, Catalog, UsbId, UsbState};

/// The schema every board's `app.json` must name.
pub const APP_SCHEMA: &str = "urn:atlas:device-app:1";
/// The schema the shared `usb-ids.json` must name.
pub const USB_IDS_SCHEMA: &str = "urn:atlas:usb-ids:1";
/// The schema of the generated catalog.
pub const CATALOG_SCHEMA: &str = "urn:atlas:catalog:1";
/// The folder under `devices/` holding one board directory each.
pub const BOARDS_DIR: &str = "boards";
/// The folder inside a board directory holding its app data.
pub const APP_DIR: &str = "app";
/// The file name of one board's app data.
pub const APP_FILE: &str = "app.json";
/// The file name of the shared USB ids, at the `devices/` root.
pub const USB_IDS_FILE: &str = "usb-ids.json";

/// A file that could not be read, or whose data is not valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    pub path: PathBuf,
    pub reason: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for LoadError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsbIdsFile {
    schema: String,
    #[serde(default)]
    ids: BTreeMap<String, UsbIdEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsbIdEntry {
    state: UsbState,
    vid: String,
    pid: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppFile {
    schema: String,
    model: String,
    display_name: String,
    #[serde(default)]
    usb: Vec<String>,
    #[serde(default)]
    network: Option<Network>,
    #[serde(default)]
    look: Option<Look>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Network {
    hostname_prefix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Look {
    picture: Option<String>,
    case: Option<String>,
}

/// Reads the app data of every board under `devices/boards/`, and the shared
/// `usb-ids.json` at the `devices/` root. A board's app data is exactly
/// `devices/boards/<board>/app/app.json`: nothing else under a board is read
/// (its Gaia description and build files included), and a board directory
/// without `app/app.json` is not in the catalog. A `devices/` folder that is
/// absent, or has no such boards, gives an empty catalog. Symlinks are not
/// followed.
pub fn from_dir(devices: &Path) -> Result<Catalog, LoadError> {
    let empty = Catalog {
        schema: CATALOG_SCHEMA.to_string(),
        boards: Vec::new(),
    };
    let boards_dir = devices.join(BOARDS_DIR);
    if !boards_dir.is_dir() {
        return Ok(empty);
    }
    let usb_ids = load_usb_ids(devices)?;
    let apps = board_apps(&boards_dir)?;

    let mut boards = Vec::new();
    for (dir_name, app) in &apps {
        boards.push(load_board(devices, dir_name, app, &usb_ids)?);
    }
    boards.sort_by(|a, b| a.model.cmp(&b.model));
    Ok(Catalog {
        schema: CATALOG_SCHEMA.to_string(),
        boards,
    })
}

/// Each board directory's `app/app.json`, with the directory's name, sorted by
/// name. A directory without one is skipped.
fn board_apps(boards_dir: &Path) -> Result<Vec<(String, PathBuf)>, LoadError> {
    let entries = fs::read_dir(boards_dir).map_err(|e| err(boards_dir, e.to_string()))?;
    let mut apps = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| err(boards_dir, e.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|e| err(boards_dir, e.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let app = entry.path().join(APP_DIR).join(APP_FILE);
        if app.is_file() {
            apps.push((entry.file_name().to_string_lossy().into_owned(), app));
        }
    }
    apps.sort();
    Ok(apps)
}

fn err(path: &Path, reason: impl Into<String>) -> LoadError {
    LoadError {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn load_usb_ids(devices: &Path) -> Result<BTreeMap<String, UsbId>, LoadError> {
    let path = devices.join(USB_IDS_FILE);
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| err(&path, e.to_string()))?;
    let file: UsbIdsFile = serde_json::from_str(&text).map_err(|e| err(&path, e.to_string()))?;
    if file.schema != USB_IDS_SCHEMA {
        return Err(err(&path, format!("schema must be {USB_IDS_SCHEMA:?}")));
    }
    let mut ids = BTreeMap::new();
    for (name, entry) in file.ids {
        if name.is_empty() || !name.chars().all(is_name_char) {
            return Err(err(
                &path,
                format!("usb id name {name:?} must be [a-z0-9-]"),
            ));
        }
        let vid = parse_id(&entry.vid).ok_or_else(|| {
            err(
                &path,
                format!("{name}: vid {:?} must be 4 hex digits", entry.vid),
            )
        })?;
        let pid = parse_id(&entry.pid).ok_or_else(|| {
            err(
                &path,
                format!("{name}: pid {:?} must be 4 hex digits", entry.pid),
            )
        })?;
        ids.insert(
            name,
            UsbId {
                state: entry.state,
                vid,
                pid,
                name: entry.name,
            },
        );
    }
    Ok(ids)
}

fn load_board(
    devices: &Path,
    dir_name: &str,
    app: &Path,
    usb_ids: &BTreeMap<String, UsbId>,
) -> Result<Board, LoadError> {
    let text = fs::read_to_string(app).map_err(|e| err(app, e.to_string()))?;
    let file: AppFile = serde_json::from_str(&text).map_err(|e| err(app, e.to_string()))?;
    if file.schema != APP_SCHEMA {
        return Err(err(app, format!("schema must be {APP_SCHEMA:?}")));
    }
    if file.model != dir_name {
        return Err(err(
            app,
            format!(
                "model {:?} must equal its board directory name {dir_name:?}",
                file.model
            ),
        ));
    }
    if file.model.is_empty() || !file.model.chars().all(is_name_char) {
        return Err(err(
            app,
            format!("model {:?} must be [a-z0-9-]", file.model),
        ));
    }
    if file.display_name.trim().is_empty() {
        return Err(err(app, "display_name is empty"));
    }
    let mut usb = Vec::new();
    let mut seen = BTreeSet::new();
    for name in &file.usb {
        let id = usb_ids
            .get(name)
            .ok_or_else(|| err(app, format!("usb id {name:?} is not in {USB_IDS_FILE}")))?;
        if !seen.insert(name) {
            return Err(err(app, format!("usb id {name:?} is listed twice")));
        }
        usb.push(id.clone());
    }
    let hostname_prefix = match file.network {
        None => None,
        Some(network) => {
            let p = network.hostname_prefix;
            if p.is_empty() || !p.chars().all(is_name_char) {
                return Err(err(app, format!("hostname_prefix {p:?} must be [a-z0-9-]")));
            }
            Some(p)
        }
    };
    // References are relative to the app/ folder, and stored relative to devices/.
    let app_dir = app.parent().unwrap_or(devices);
    let rel_dir = app_dir.strip_prefix(devices).unwrap_or(app_dir);
    let (picture, case) = match file.look {
        None => (None, None),
        Some(look) => (
            look.picture
                .map(|p| reference(app, rel_dir, &p))
                .transpose()?,
            look.case.map(|c| reference(app, rel_dir, &c)).transpose()?,
        ),
    };
    Ok(Board {
        model: file.model,
        display_name: file.display_name,
        usb,
        hostname_prefix,
        picture,
        case,
    })
}

/// A file a board names, relative to its app/ folder, becomes a path relative
/// to the `devices/` root. It must stay inside that folder.
fn reference(app: &Path, rel_dir: &Path, name: &str) -> Result<String, LoadError> {
    let path = Path::new(name);
    let inside = !name.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)));
    if !inside {
        return Err(err(
            app,
            format!("look reference {name:?} must be a path inside the board's app/ folder"),
        ));
    }
    Ok(rel_dir.join(path).to_string_lossy().replace('\\', "/"))
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// Exactly four hex digits, as a USB id is written.
fn parse_id(text: &str) -> Option<u16> {
    if text.len() != 4 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(text, 16).ok()
}
