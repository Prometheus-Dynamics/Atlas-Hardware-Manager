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

use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::rockchip::crc::rk_crc32;
use crate::rockchip::loader::{ENTRY_FIELDS, HEADER_SIZE, MAGIC_LDR};
use crate::rockchip::rc4::{rc4, rc4_blocks};

/// boot_merger's polynomial (the C table's second entry is 0x04C10DB7).
const POLYNOMIAL: u32 = 0x04c1_0db7;
/// `ENTRY_ALIGN`: every part is zero-padded to this.
const ENTRY_ALIGN: usize = 2048;
const SECTOR: usize = 512;
/// What the newer boot_merger writes for `mergerVersion` (the C source says
/// 0x01030000).
const MERGER_VERSION: u32 = 0x0100_0000;
/// Name characters an entry holds (`MAX_NAME_LEN`).
const NAME_UNITS: usize = 20;
/// RKNS header: the images table, its size field, and where the hash goes.
const RKNS_SIZE: usize = 2048;
const RKNS_IMAGES: usize = 120;
const RKNS_IMAGE_SIZE: usize = 88;
const RKNS_MAX_IMAGES: usize = 4;
const RKNS_HASH_AT: usize = 1536;

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
    /// Settings that change the output and that nothing settles.
    unsupported: Vec<String>,
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
        let mut ini = Self::default();
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
            if name.starts_with("LOADER") && name.ends_with("_PARAM") {
                ini.unsupported
                    .push(format!("[{name}] (load address and flag)"));
            } else if !known {
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
        // Keys the newer boot_merger reads and no source or published file
        // shows the effect of: refused at merge, not guessed.
        for (section, key) in [("SYSTEM", "ALIGN"), ("FLAG", "CRC_OFF")] {
            if find(section, key).is_some() {
                ini.unsupported.push(format!("[{section}] {key}"));
            }
        }
        for (_, entries) in sections.iter().filter(|s| s.0 == "BOOT1_PARAM") {
            for (line, key, value) in entries {
                let zero = value
                    .trim_start_matches("0x")
                    .chars()
                    .all(|digit| digit == '0');
                if !zero {
                    ini.unsupported
                        .push(format!("[BOOT1_PARAM] {key}={value} (line {line})"));
                }
            }
        }
        Ok(ini)
    }
}

/// The header's release time (the C reads the build machine's local clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl ReleaseTime {
    /// UTC from Unix seconds (the C uses `localtime`; the caller picks).
    pub fn from_unix_utc(seconds: u64) -> Self {
        let (days, rest) = (seconds / 86_400, seconds % 86_400);
        // Howard Hinnant's civil_from_days.
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: year as u16,
            month,
            day,
            hour: (rest / 3_600) as u8,
            minute: (rest % 3_600 / 60) as u8,
            second: (rest % 60) as u8,
        }
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

fn pad_align(data: &[u8]) -> Vec<u8> {
    let mut padded = data.to_vec();
    padded.resize(data.len().div_ceil(ENTRY_ALIGN) * ENTRY_ALIGN, 0);
    padded
}

/// The RKNS header `rkcommon_set_header0_v2` writes for `images` (each
/// already padded): images from sector 4 on, SHA-256 of every image, then
/// SHA-256 of the header up to the hash field.
fn rkns_header(images: &[&[u8]]) -> Result<Vec<u8>, MergeError> {
    if images.len() > RKNS_MAX_IMAGES {
        return Err(MergeError::Unsupported(format!(
            "{} images; an RKNS header holds {RKNS_MAX_IMAGES}",
            images.len()
        )));
    }
    let mut header = vec![0u8; RKNS_SIZE];
    header[..4].copy_from_slice(b"RKNS");
    // [31:16] images, [15:0] the hash field's offset in words... 384.
    let size_and_nimage = ((images.len() as u32) << 16) + 384;
    header[8..12].copy_from_slice(&size_and_nimage.to_le_bytes());
    header[12..16].copy_from_slice(&1u32.to_le_bytes()); // boot_flag: SHA-256
    let mut sector = 4u32;
    for (index, image) in images.iter().enumerate() {
        let sectors = u32::try_from(image.len() / SECTOR)
            .ok()
            .filter(|count| *count <= 0xffff)
            .ok_or_else(|| MergeError::OutOfRange("an image is over 32 MiB".into()))?;
        let at = RKNS_IMAGES + index * RKNS_IMAGE_SIZE;
        header[at..at + 4].copy_from_slice(&((sectors << 16) + sector).to_le_bytes());
        header[at + 4..at + 8].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        header[at + 12..at + 16].copy_from_slice(&(index as u32 + 1).to_le_bytes());
        header[at + 24..at + 56].copy_from_slice(&Sha256::digest(image));
        sector += sectors;
    }
    let hash = Sha256::digest(&header[..RKNS_HASH_AT]);
    header[RKNS_HASH_AT..RKNS_HASH_AT + 32].copy_from_slice(&hash);
    Ok(header)
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

/// Merges the loader `ini` describes; `read` returns the bytes of a part
/// from its ini path. Only the `NEWIDB` layout is built: the older one (no
/// UsbHead/FlashHead) was never compared to a published file.
///
/// UNCONFIRMED, from the C only: `471_RC4_OFF=false` scrambles each 471/472
/// stage (and the UsbHead) in one RC4 stream; `RC4_OFF=false` clears the
/// header's `rc4Flag` and changes nothing else. More than two 471+472
/// stages in the UsbHead, and three loader stages in the FlashHead (RK3576
/// has FlashBoost), follow the same RKNS rules but no published file shows
/// them.
pub fn merge(
    ini: &Ini,
    time: ReleaseTime,
    mut read: impl FnMut(&str) -> io::Result<Vec<u8>>,
) -> Result<Vec<u8>, MergeError> {
    if !ini.newidb {
        return Err(MergeError::Unsupported(
            "an ini without [SYSTEM] NEWIDB=true (the legacy layout is not built)".into(),
        ));
    }
    if let Some(first) = ini.unsupported.first() {
        return Err(MergeError::Unsupported(format!(
            "{first}: its effect on the loader isn't settled by any source or published file"
        )));
    }
    let chip = chip_type(&ini.chip)?;
    let mut part = |path: &str| -> Result<Vec<u8>, MergeError> {
        let data = read(path).map_err(|error| MergeError::MissingPart {
            path: path.to_string(),
            reason: error.to_string(),
        })?;
        if data.is_empty() {
            return Err(MergeError::EmptyPart(path.to_string()));
        }
        Ok(pad_align(&data))
    };
    let mut stage = |paths: &[String]| -> Result<Vec<NamedPart>, MergeError> {
        paths
            .iter()
            .map(|path| Ok((entry_name(path), part(path)?)))
            .collect()
    };
    let (ddr, usbplug) = (stage(&ini.code471)?, stage(&ini.code472)?);
    let mut loaders = Vec::new();
    for (name, path) in &ini.loaders {
        loaders.push((entry_name(name), part(path)?));
    }

    let scramble_stage = |data: Vec<u8>| if ini.rc4_off_471 { data } else { rc4(&data) };
    let usb_images: Vec<&[u8]> = ddr.iter().chain(&usbplug).map(|p| p.1.as_slice()).collect();
    let usb_head = scramble_stage(rkns_header(&usb_images)?);
    let flash_images: Vec<&[u8]> = loaders.iter().map(|p| p.1.as_slice()).collect();
    let flash_head = rc4_blocks(&rkns_header(&flash_images)?);

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
    out[10..14].copy_from_slice(&MERGER_VERSION.to_le_bytes());
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
    Ok(out)
}

/// [`merge`] with the parts read from `base_dir` (the ini's paths are
/// relative to rkbin's root).
pub fn merge_dir(ini: &Ini, time: ReleaseTime, base_dir: &Path) -> Result<Vec<u8>, MergeError> {
    merge(ini, time, |path| std::fs::read(base_dir.join(path)))
}
