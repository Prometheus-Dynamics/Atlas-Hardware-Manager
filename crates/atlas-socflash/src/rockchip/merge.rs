//! A native boot_merger: Rockchip's RKBOOT ini and the vendor parts (DDR
//! init, usbplug, SPL) in, the merged loader file out (the file
//! [`Loader::parse`](crate::rockchip::loader::Loader::parse) reads).
//!
//! Sources, all pinned in `atlas-blobs/merger-src/SOURCES.txt`:
//! - rockchip-linux/u-boot 1c535d6 `tools/rockchip/boot_merger.c` (the only
//!   boot_merger source Rockchip publishes) for the header and entry
//!   structs (`boot_merger.h` 141-179, `getBoothdr` 753), 512/2048 alignment (`writeFile` 612-663,
//!   `saveEntry`), the RC4 of loader entries per 512 bytes (`writeFile` 612-663,
//!   `P_RC4`), the entry order and the trailing CRC-32 (`CRC_32`, table
//!   0x04C10DB7, from 0, MSB first, the file before it, little endian);
//! - rockchip-linux/u-boot 1c535d6 `tools/rkcommon.c` for the RKNS header
//!   (`rkcommon_set_header0_v2`) that the `NEWIDB` loaders carry as their
//!   UsbHead and FlashHead entries.
//!
//! The boot_merger rkbin ships (`tools/boot_merger`) is newer than that
//! source and has none of its own: a loader it wrote differs from what the C
//! says in the magic ("LDR "), the version (plain bytes, not BCD), the merge
//! version (0x01000000), a reserved byte, the extra UsbHead/FlashHead
//! entries, 20-character names cut at the first dot, and the 471/472 stages
//! left unscrambled with `471_RC4_OFF`. Those were read off Radxa's
//! published rk3588_spl_loader_v1.08.111.bin and rebuilt byte for byte from
//! its rkbin parts (`tests/merge_vectors.rs`). What no published file shows
//! is marked UNCONFIRMED below.
//!
//! Gaia's loaders from boot_merger 1.35 and 1.38 (rock-5c rk3588, rock-4d
//! rk3576, zero-3w rk356x, luckfox rv1106) add, all read
//! off those files and absent from the C: `ALIGN`, `LOADERn_PARAM`,
//! `BOOT1_PARAM` words, `CREATE_IDB`/`IDB_PATH` (see [`merge_full`]) and the
//! header's byte 46. They are rebuilt byte for byte in
//! `tests/gaia_merged.rs`.

use std::io;
use std::path::Path;

use crate::rockchip::crc::rk_crc32;
use crate::rockchip::loader::{ENTRY_FIELDS, HEADER_SIZE, MAGIC_LDR};
use crate::rockchip::merge_rkns::{BOOT1_WORDS, ImageParam, SECTOR, rkns_header};
pub use crate::rockchip::merge_time::ReleaseTime;
use crate::rockchip::rc4::{rc4, rc4_blocks};

/// boot_merger's polynomial (the C table's second entry is 0x04C10DB7).
const POLYNOMIAL: u32 = 0x04c1_0db7;
/// `ENTRY_ALIGN`: every part is zero-padded to this.
const ENTRY_ALIGN: usize = 2048;
/// Name characters an entry holds (`MAX_NAME_LEN`).
const NAME_UNITS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    /// The ini can't be read as boot_merger reads it.
    Ini(String),
    /// A part the ini names can't be read.
    MissingPart { path: String, reason: String },
    /// A part has no bytes (boot_merger's size arithmetic underflows).
    EmptyPart(String),
    /// The chip name doesn't fit the header's four characters after "RK".
    ChipTooLong(String),
    /// A value outside its field.
    OutOfRange(String),
    /// An ini setting no source or published file settles.
    Unsupported(String),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ini(message) => write!(f, "bad RKBOOT ini: {message}"),
            Self::MissingPart { path, reason } => write!(f, "can't read part {path}: {reason}"),
            Self::EmptyPart(path) => write!(f, "part {path} is empty"),
            Self::ChipTooLong(chip) => write!(
                f,
                "chip name {chip:?} is too long: the header holds four characters after the first two"
            ),
            Self::OutOfRange(message) => write!(f, "{message}"),
            Self::Unsupported(message) => write!(f, "not supported: {message}"),
        }
    }
}

impl std::error::Error for MergeError {}

/// The RKBOOT ini, as far as boot_merger reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ini {
    /// `[CHIP_NAME] NAME`, e.g. RK3588.
    pub chip: String,
    pub major: u8,
    pub minor: u8,
    /// `[CODE471_OPTION]` `Path<n>` values (DDR init), in order.
    pub code471: Vec<String>,
    /// `Sleep`: milliseconds the host waits after each 471 stage.
    pub sleep471: u32,
    /// `[CODE472_OPTION]` (usbplug).
    pub code472: Vec<String>,
    pub sleep472: u32,
    /// `[LOADER_OPTION]`: stage name and the path of its part, in order.
    pub loaders: Vec<(String, String)>,
    /// `[OUTPUT] PATH`.
    pub output: String,
    /// `[SYSTEM] NEWIDB`: the RKNS UsbHead/FlashHead layout.
    pub newidb: bool,
    /// `[FLAG] 471_RC4_OFF`: the 471 and 472 stages stay unscrambled.
    pub rc4_off_471: bool,
    /// `[FLAG] RC4_OFF`: the header's `rc4Flag`.
    pub rc4_off: bool,
    /// `[SYSTEM] ALIGN`, in 512-byte sectors: the loader (flash) entries and
    /// the FlashHead are padded to this; 4 (2048 bytes) when absent.
    pub align_sectors: u32,
    /// `[FLAG] CREATE_IDB`: also build the idblock.
    pub create_idb: bool,
    /// `[OUTPUT] IDB_PATH`.
    pub idb_path: Option<String>,
    /// `[FLAG] CRC_OFF`: sets header byte 46 (see `merge_full`). The trailing
    /// CRC-32 is written either way (rock-4d rk3576_loader_v1.13.109.bin).
    pub crc_off: bool,
    /// `[LOADERn_PARAM]`, one per loader stage in order.
    pub loader_params: Vec<ImageParam>,
    /// `[BOOT1_PARAM] WORD_0..WORD_7`.
    pub boot1_words: [u32; BOOT1_WORDS],
    /// The header's merge version (0x01000000 in every loader seen, from
    /// boot_merger 1.35 and 1.38 alike).
    pub merger_version: u32,
}

/// A section name and its (line, key, value) settings.
type Section = (String, Vec<(usize, String, String)>);

fn ini_error<T>(line: usize, message: impl std::fmt::Display) -> Result<T, MergeError> {
    Err(MergeError::Ini(format!("line {line}: {message}")))
}

fn flag(line: usize, value: &str) -> Result<bool, MergeError> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => ini_error(line, format!("{other:?} isn't true or false")),
    }
}

fn number(line: usize, key: &str, value: &str) -> Result<u32, MergeError> {
    match value.trim().parse::<u32>() {
        Ok(number) => Ok(number),
        Err(_) => ini_error(line, format!("{key} is {value:?}, not a number")),
    }
}

/// A number as the ini writes it: decimal or 0x hex.
fn word(line: usize, key: &str, value: &str) -> Result<u32, MergeError> {
    let value = value.trim();
    let parsed = match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => value.parse::<u32>(),
    };
    parsed.or_else(|_| ini_error(line, format!("{key} is {value:?}, not a number")))
}

/// `LOADER<n>_PARAM` -> n.
fn loader_param_index(section: &str) -> Option<usize> {
    let digits = section.strip_prefix("LOADER")?.strip_suffix("_PARAM")?;
    digits.parse().ok().filter(|n| *n >= 1)
}

/// fixPath: backslashes become slashes.
fn fix_path(value: &str) -> String {
    value.replace('\\', "/")
}

impl Ini {
    /// Reads the ini the way boot_merger does: `[SECTION]` lines, `KEY=value`
    /// lines, `#` comments, unknown sections refused. `Path<n>`, `LOADER<n>`
    /// and the `<name>=path` lines are looked up by number and name, so their
    /// order doesn't matter; like the C, every `<n>` from 1 to NUM must exist.
    pub fn parse(text: &str) -> Result<Self, MergeError> {
        let mut ini = Self {
            merger_version: 0x0100_0000,
            ..Self::default()
        };
        let mut sections: Vec<Section> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(name) = trimmed.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                sections.push((name.to_string(), Vec::new()));
            } else if let Some((key, value)) = trimmed.split_once('=') {
                let Some(section) = sections.last_mut() else {
                    return ini_error(line, "a setting before any [SECTION]");
                };
                section
                    .1
                    .push((line, key.trim().to_string(), value.trim().to_string()));
            } else {
                return ini_error(
                    line,
                    format!("{trimmed:?} is neither [SECTION] nor KEY=value"),
                );
            }
        }
        let find = |section: &str, key: &str| -> Option<(usize, &str)> {
            sections
                .iter()
                .filter(|s| s.0 == section)
                .flat_map(|s| s.1.iter())
                .rev()
                .find(|entry| entry.1 == key)
                .map(|entry| (entry.0, entry.2.as_str()))
        };
        let need = |section: &str, key: &str| -> Result<(usize, &str), MergeError> {
            find(section, key)
                .ok_or_else(|| MergeError::Ini(format!("[{section}] has no {key} (or no section)")))
        };
        for (name, entries) in &sections {
            let known = matches!(
                name.as_str(),
                "CHIP_NAME"
                    | "VERSION"
                    | "CODE471_OPTION"
                    | "CODE472_OPTION"
                    | "LOADER_OPTION"
                    | "OUTPUT"
                    | "SYSTEM"
                    | "FLAG"
                    | "BOOT1_PARAM"
            );
            if !known && loader_param_index(name).is_none() {
                return ini_error(
                    entries.first().map_or(0, |e| e.0).saturating_sub(1),
                    format!("unknown section [{name}]"),
                );
            }
        }

        ini.chip = need("CHIP_NAME", "NAME")?.1.to_string();
        let (line, major) = need("VERSION", "MAJOR")?;
        let major = number(line, "MAJOR", major)?;
        let (line, minor) = need("VERSION", "MINOR")?;
        let minor = number(line, "MINOR", minor)?;
        let (Ok(major), Ok(minor)) = (u8::try_from(major), u8::try_from(minor)) else {
            return Err(MergeError::OutOfRange(format!(
                "version {major}.{minor} doesn't fit the header's one byte each"
            )));
        };
        (ini.major, ini.minor) = (major, minor);

        for (section, paths, sleep) in [
            ("CODE471_OPTION", &mut ini.code471, &mut ini.sleep471),
            ("CODE472_OPTION", &mut ini.code472, &mut ini.sleep472),
        ] {
            let Some((line, count)) = find(section, "NUM") else {
                continue; // the C starts with code471Ok = code472Ok = true
            };
            for n in 1..=number(line, "NUM", count)? {
                paths.push(fix_path(need(section, &format!("Path{n}"))?.1));
            }
            if let Some((line, value)) = find(section, "Sleep") {
                *sleep = number(line, "Sleep", value)?;
            }
        }

        let (line, count) = match find("LOADER_OPTION", "NUM") {
            Some(found) => found,
            None => need("LOADER_OPTION", "LOADERCOUNT")?,
        };
        let count = number(line, "NUM", count)?;
        if count == 0 {
            return ini_error(line, "NUM is 0: a loader needs FlashData and FlashBoot");
        }
        for n in 1..=count {
            let name = need("LOADER_OPTION", &format!("LOADER{n}"))?.1;
            let path = need("LOADER_OPTION", name)?.1;
            ini.loaders.push((name.to_string(), fix_path(path)));
        }
        ini.output = need("OUTPUT", "PATH")?.1.to_string();

        if let Some((line, value)) = find("SYSTEM", "NEWIDB") {
            ini.newidb = flag(line, value)?;
        }
        for (key, field) in [
            ("471_RC4_OFF", &mut ini.rc4_off_471),
            ("RC4_OFF", &mut ini.rc4_off),
        ] {
            if let Some((line, value)) = find("FLAG", key) {
                *field = flag(line, value)?;
            }
        }
        ini.align_sectors = match find("SYSTEM", "ALIGN") {
            Some((line, value)) => word(line, "ALIGN", value)?,
            None => (ENTRY_ALIGN / SECTOR) as u32,
        };
        if ini.align_sectors == 0 || ini.align_sectors > 0xffff {
            return Err(MergeError::OutOfRange("ALIGN is 0 or over 65535".into()));
        }
        for (key, field) in [
            ("CREATE_IDB", &mut ini.create_idb),
            ("CRC_OFF", &mut ini.crc_off),
        ] {
            if let Some((line, value)) = find("FLAG", key) {
                *field = flag(line, value)?;
            }
        }
        ini.idb_path = find("OUTPUT", "IDB_PATH").map(|(_, path)| path.to_string());
        for (_, entries) in sections.iter().filter(|s| s.0 == "BOOT1_PARAM") {
            for (line, key, value) in entries {
                let index = key
                    .strip_prefix("WORD_")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| *n < BOOT1_WORDS);
                match index {
                    Some(n) => ini.boot1_words[n] = word(*line, key, value)?,
                    None => return ini_error(*line, format!("unknown [BOOT1_PARAM] key {key}")),
                }
            }
        }
        ini.loader_params = vec![ImageParam::default(); ini.loaders.len()];
        for (name, entries) in &sections {
            let Some(n) = loader_param_index(name) else {
                continue;
            };
            let Some(param) = ini.loader_params.get_mut(n - 1) else {
                return ini_error(
                    entries.first().map_or(0, |e| e.0),
                    format!("[{name}] but only {} loaders", ini.loaders.len()),
                );
            };
            for (line, key, value) in entries {
                match key.as_str() {
                    "LOAD_ADDR" => param.load_addr = word(*line, key, value)?,
                    "FLAG" => param.flag = word(*line, key, value)?,
                    _ => return ini_error(*line, format!("unknown [{name}] key {key}")),
                }
            }
        }
        Ok(ini)
    }
}

impl Ini {
    /// What the Luckfox `scripts/spl.sh` does with `sed`: the `FlashBoot`
    /// part becomes `path`. False when the ini has no FlashBoot stage.
    pub fn set_flash_boot(&mut self, path: &str) -> bool {
        let slot = self
            .loaders
            .iter_mut()
            .find(|(name, _)| name == "FlashBoot");
        slot.map(|(_, part)| *part = fix_path(path)).is_some()
    }
}

/// `getName`: the file name up to its first dot (the C cuts at the last, the
/// newer tool at the first: rk3588_usbplug_v1.09.bin is "rk3588_usbplug_v1"
/// in the published loader), at most 20 characters, no NUL needed.
fn entry_name(path_or_name: &str) -> Vec<u16> {
    let file = path_or_name.rsplit('/').next().unwrap_or(path_or_name);
    let stem = file.split('.').next().unwrap_or(file);
    stem.encode_utf16().take(NAME_UNITS).collect()
}

/// An entry name and its padded part.
type NamedPart = (Vec<u16>, Vec<u8>);

struct Stored {
    code: u32,
    name: Vec<u16>,
    data: Vec<u8>,
    delay: u32,
}

/// `convertChipType(chip + 2)`: four characters, first in the high byte.
fn chip_type(chip: &str) -> Result<u32, MergeError> {
    let digits = chip
        .get(2..)
        .filter(|rest| chip.is_ascii() && !rest.is_empty());
    let Some(digits) = digits else {
        return Err(MergeError::Ini(format!(
            "chip name {chip:?} isn't RK plus a number"
        )));
    };
    if digits.len() > 4 {
        return Err(MergeError::ChipTooLong(chip.to_string()));
    }
    let mut bytes = [0u8; 4];
    bytes[..digits.len()].copy_from_slice(digits.as_bytes());
    Ok(u32::from_be_bytes(bytes))
}

/// What a merge makes: the loader, and the idblock when `CREATE_IDB` is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    pub loader: Vec<u8>,
    pub idblock: Option<Vec<u8>>,
}

fn pad_to(data: &[u8], unit: usize) -> Vec<u8> {
    let mut padded = data.to_vec();
    padded.resize(data.len().div_ceil(unit) * unit, 0);
    padded
}

/// [`merge_full`]'s loader only.
pub fn merge(
    ini: &Ini,
    time: ReleaseTime,
    read: impl FnMut(&str) -> io::Result<Vec<u8>>,
) -> Result<Vec<u8>, MergeError> {
    Ok(merge_full(ini, time, read)?.loader)
}

/// Merges the loader `ini` describes; `read` returns the bytes of a part
/// from its ini path. Only the `NEWIDB` layout is built: the older one (no
/// UsbHead/FlashHead) was never compared to a published file.
///
/// The 471 and 472 stages are padded to 2048 bytes; the FlashHead and the
/// loader stages to `ALIGN` sectors (4096 with `ALIGN=8`, as in Gaia's
/// rock-4d loader). The idblock, when `CREATE_IDB` is set, is the FlashHead
/// and the loader stages as plain bytes, back to back (read off the rock-4d
/// and rv1106 idblocks, which equal the loader's flash entries
/// unscrambled). `CRC_OFF` leaves the trailing CRC-32 in place.
///
/// UNCONFIRMED, from the C only: `471_RC4_OFF=false` scrambles each 471/472
/// stage (and the UsbHead) in one RC4 stream; `RC4_OFF=false` clears the
/// header's `rc4Flag` and changes nothing else.
pub fn merge_full(
    ini: &Ini,
    time: ReleaseTime,
    mut read: impl FnMut(&str) -> io::Result<Vec<u8>>,
) -> Result<Merged, MergeError> {
    if !ini.newidb {
        return Err(MergeError::Unsupported(
            "an ini without [SYSTEM] NEWIDB=true (the legacy layout is not built)".into(),
        ));
    }
    let chip = chip_type(&ini.chip)?;
    let align = ini.align_sectors as usize * SECTOR;
    let mut part = |path: &str, unit: usize| -> Result<Vec<u8>, MergeError> {
        let data = read(path).map_err(|error| MergeError::MissingPart {
            path: path.to_string(),
            reason: error.to_string(),
        })?;
        if data.is_empty() {
            return Err(MergeError::EmptyPart(path.to_string()));
        }
        Ok(pad_to(&data, unit))
    };
    let mut stage = |paths: &[String]| -> Result<Vec<NamedPart>, MergeError> {
        paths
            .iter()
            .map(|path| Ok((entry_name(path), part(path, ENTRY_ALIGN)?)))
            .collect()
    };
    let (ddr, usbplug) = (stage(&ini.code471)?, stage(&ini.code472)?);
    let mut loaders = Vec::new();
    for (name, path) in &ini.loaders {
        loaders.push((entry_name(name), part(path, align)?));
    }

    let scramble_stage = |data: Vec<u8>| if ini.rc4_off_471 { data } else { rc4(&data) };
    let usb_images: Vec<&[u8]> = ddr.iter().chain(&usbplug).map(|p| p.1.as_slice()).collect();
    let usb_head = rkns_header(&usb_images, ENTRY_ALIGN, &ini.boot1_words, &[])?;
    let usb_head = scramble_stage(usb_head);
    let flash_images: Vec<&[u8]> = loaders.iter().map(|p| p.1.as_slice()).collect();
    let flash_plain = rkns_header(&flash_images, align, &ini.boot1_words, &ini.loader_params)?;
    let flash_head = rc4_blocks(&flash_plain);
    let idblock = ini.create_idb.then(|| {
        let mut idb = flash_plain;
        loaders
            .iter()
            .for_each(|(_, data)| idb.extend_from_slice(data));
        idb
    });

    let mut stored = vec![Stored {
        code: 1,
        name: entry_name("UsbHead"),
        data: usb_head,
        delay: ini.sleep471,
    }];
    for (name, data) in ddr {
        stored.push(Stored {
            code: 1,
            name,
            data: scramble_stage(data),
            delay: ini.sleep471,
        });
    }
    for (name, data) in usbplug {
        stored.push(Stored {
            code: 2,
            name,
            data: scramble_stage(data),
            delay: ini.sleep472,
        });
    }
    stored.push(Stored {
        code: 4,
        name: entry_name("FlashHead"),
        data: flash_head,
        delay: 0,
    });
    for (name, data) in loaders {
        // writeFile(fix = true): the loader stages are scrambled per 512
        // bytes whatever the header's flag says.
        stored.push(Stored {
            code: 4,
            name,
            data: rc4_blocks(&data),
            delay: 0,
        });
    }

    let count = |code: u32| stored.iter().filter(|s| s.code == code).count();
    let (n471, n472, nloader) = (count(1), count(2), count(4));
    let counts = [n471, n472, nloader];
    if counts.iter().any(|n| *n > 255) {
        return Err(MergeError::OutOfRange(
            "over 255 entries of one kind".into(),
        ));
    }
    let table_at = |before: usize| (HEADER_SIZE + before * ENTRY_FIELDS) as u32;
    let mut out = vec![0u8; HEADER_SIZE];
    out[0..4].copy_from_slice(&MAGIC_LDR);
    out[4..6].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    out[6..10].copy_from_slice(&((u32::from(ini.major) << 8) | u32::from(ini.minor)).to_le_bytes());
    out[10..14].copy_from_slice(&ini.merger_version.to_le_bytes());
    out[14..16].copy_from_slice(&time.year.to_le_bytes());
    out[16..21].copy_from_slice(&[time.month, time.day, time.hour, time.minute, time.second]);
    out[21..25].copy_from_slice(&chip.to_le_bytes());
    for (index, (at, before)) in [(25, 0), (31, n471), (37, n471 + n472)]
        .into_iter()
        .enumerate()
    {
        out[at] = counts[index] as u8;
        out[at + 1..at + 5].copy_from_slice(&table_at(before).to_le_bytes());
        out[at + 5] = ENTRY_FIELDS as u8;
    }
    out[43] = 0; // signFlag: unsigned
    out[44] = u8::from(ini.rc4_off);
    out[45] = 1; // reserved[0]: set in every NEWIDB loader seen
    // reserved[1] is 1 in the one loader made with CRC_OFF (rock-4d), 0 in
    // the four without. That loader also has ALIGN=8 and LOADERn_PARAM, and
    // no file has either key without CRC_OFF, so the reference can't tell
    // which key sets it: CRC_OFF is the guess.
    out[46] = u8::from(ini.crc_off);
    let mut data_at = HEADER_SIZE + stored.len() * ENTRY_FIELDS;
    for entry in &stored {
        let mut record = vec![0u8; ENTRY_FIELDS];
        record[0] = ENTRY_FIELDS as u8;
        record[1..5].copy_from_slice(&entry.code.to_le_bytes());
        for (index, unit) in entry.name.iter().enumerate() {
            record[5 + index * 2..7 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        record[45..49].copy_from_slice(&(data_at as u32).to_le_bytes());
        record[49..53].copy_from_slice(&(entry.data.len() as u32).to_le_bytes());
        record[53..57].copy_from_slice(&entry.delay.to_le_bytes());
        out.extend_from_slice(&record);
        data_at += entry.data.len();
    }
    for entry in &stored {
        out.extend_from_slice(&entry.data);
    }
    let crc = rk_crc32(&out, POLYNOMIAL);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(Merged {
        loader: out,
        idblock,
    })
}

/// [`merge`] with the parts read from `base_dir` (the ini's paths are
/// relative to rkbin's root).
pub fn merge_dir(ini: &Ini, time: ReleaseTime, base_dir: &Path) -> Result<Vec<u8>, MergeError> {
    merge(ini, time, |path| std::fs::read(base_dir.join(path)))
}

/// [`merge_full`] with the parts read from `base_dir`.
pub fn merge_full_dir(ini: &Ini, time: ReleaseTime, base_dir: &Path) -> Result<Merged, MergeError> {
    merge_full(ini, time, |path| std::fs::read(base_dir.join(path)))
}
