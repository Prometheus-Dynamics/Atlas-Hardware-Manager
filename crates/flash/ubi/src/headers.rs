//! UBI's on-flash headers (`drivers/mtd/ubi/ubi-media.h`): the EC header at
//! the start of every PEB, the VID header of every PEB holding data, and the
//! volume-table records the layout volume holds. Every field is big-endian,
//! and every CRC is [`crate::crc::ubi_crc`] over the bytes before it.
//!
//! `decode` checks what makes bytes a header: magic, version, CRC and volume
//! type. `validate` checks the fields the way the kernel's `validate_ec_hdr`,
//! `validate_vid_hdr` and `vtbl_check` do, as far as a file can check them.

use crate::crc::ubi_crc;
use std::fmt;

/// The EC header magic, "UBI#".
pub const EC_MAGIC: u32 = 0x5542_4923;
/// The VID header magic, "UBI!".
pub const VID_MAGIC: u32 = 0x5542_4921;
/// The on-media format version (`UBI_VERSION`).
pub const VERSION: u8 = 1;
/// The bytes of an EC or VID header.
pub const HEADER_SIZE: usize = 64;
/// The bytes of a header its CRC covers: all but the CRC itself.
pub const HEADER_CRC_SIZE: usize = 60;
/// The bytes of one volume-table record.
pub const VTBL_RECORD_SIZE: usize = 172;
/// The bytes of a volume-table record its CRC covers.
pub const VTBL_RECORD_CRC_SIZE: usize = VTBL_RECORD_SIZE - 4;
/// The most volumes a UBI device holds (`UBI_MAX_VOLUMES`).
pub const MAX_VOLUMES: u32 = 128;
/// The longest volume name, in bytes (`UBI_VOL_NAME_MAX`).
pub const VOLUME_NAME_MAX: usize = 127;
/// The layout volume's id (`UBI_LAYOUT_VOLUME_ID`).
pub const LAYOUT_VOLUME_ID: u32 = 0x7fff_efff;
/// The layout volume's LEBs: two copies of the volume table.
pub const LAYOUT_VOLUME_LEBS: u32 = 2;
/// `UBI_COMPAT_REJECT`: the layout volume's compat, which an older UBI
/// refuses to attach.
pub const LAYOUT_VOLUME_COMPAT: u8 = 5;
/// The volume-table flag for the volume UBI grows over free PEBs on first
/// attach (`UBI_VTBL_AUTORESIZE_FLG`).
pub const VTBL_AUTORESIZE: u8 = 0x01;
/// The PEBs UBI keeps for itself beyond the layout volume: one for wear
/// levelling (`WL_RESERVED_PEBS`) and one for atomic LEB changes
/// (`EBA_RESERVED_PEBS`).
pub const RUNTIME_RESERVED_PEBS: u64 = 2;
/// Erase counters above this are corrupt, not worn (`UBI_MAX_ERASECOUNTER`).
pub const MAX_ERASE_COUNTER: u64 = 0x7fff_ffff;
/// An erased flash byte.
pub const ERASED: u8 = 0xff;

/// A volume's type, in VID headers and volume-table records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VolumeKind {
    /// `UBI_VID_DYNAMIC`: read-write, no per-LEB size or CRC.
    Dynamic,
    /// `UBI_VID_STATIC`: read-only, each LEB carries its size and CRC.
    Static,
}

impl VolumeKind {
    pub fn code(self) -> u8 {
        match self {
            Self::Dynamic => 1,
            Self::Static => 2,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Dynamic),
            2 => Some(Self::Static),
            _ => None,
        }
    }
}

/// Why bytes are not a usable header or record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderError {
    /// Fewer bytes than the header.
    TooShort {
        what: &'static str,
        len: usize,
    },
    Magic {
        what: &'static str,
        found: u32,
    },
    Version {
        what: &'static str,
        found: u8,
    },
    Crc {
        what: &'static str,
        stored: u32,
        computed: u32,
    },
    VolumeType {
        what: &'static str,
        code: u8,
    },
    /// A volume name of 0 or more than [`VOLUME_NAME_MAX`] bytes.
    NameLength(usize),
    NameNotUtf8,
    /// A field the kernel would refuse.
    Field {
        what: &'static str,
        field: &'static str,
        value: u64,
    },
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { what, len } => write!(f, "{what} cut short at {len} bytes"),
            Self::Magic { what, found } => write!(f, "{what} magic {found:#010x}"),
            Self::Version { what, found } => write!(f, "{what} version {found}"),
            Self::Crc {
                what,
                stored,
                computed,
            } => {
                write!(f, "{what} CRC {stored:#010x}, computed {computed:#010x}")
            }
            Self::VolumeType { what, code } => write!(f, "{what} volume type {code}"),
            Self::NameLength(len) => write!(f, "volume table record name length {len}"),
            Self::NameNotUtf8 => f.write_str("volume table record name isn't UTF-8"),
            Self::Field { what, field, value } => write!(f, "{what} {field} {value}"),
        }
    }
}

impl std::error::Error for HeaderError {}

const EC: &str = "EC header";
const VID: &str = "VID header";
const VTBL: &str = "volume table record";
const LAYOUT: &str = "PEB layout";

fn field(what: &'static str, field: &'static str, value: impl Into<u64>) -> HeaderError {
    HeaderError::Field {
        what,
        field,
        value: value.into(),
    }
}

pub(crate) fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn be64(bytes: &[u8], at: usize) -> u64 {
    (u64::from(be32(bytes, at)) << 32) | u64::from(be32(bytes, at + 4))
}

/// Erased flash: the header's every byte 0xff.
fn erased(bytes: &[u8]) -> bool {
    bytes.iter().take(HEADER_SIZE).all(|byte| *byte == ERASED)
}

/// Checks the magic, version and CRC of a header-sized prefix of `bytes`.
fn check(bytes: &[u8], magic: u32, what: &'static str) -> Result<(), HeaderError> {
    if bytes.len() < HEADER_SIZE {
        return Err(HeaderError::TooShort {
            what,
            len: bytes.len(),
        });
    }
    let found = be32(bytes, 0);
    if found != magic {
        return Err(HeaderError::Magic { what, found });
    }
    if bytes[4] != VERSION {
        return Err(HeaderError::Version {
            what,
            found: bytes[4],
        });
    }
    let computed = ubi_crc(&bytes[..HEADER_CRC_SIZE]);
    let stored = be32(bytes, HEADER_CRC_SIZE);
    if computed != stored {
        return Err(HeaderError::Crc {
            what,
            stored,
            computed,
        });
    }
    Ok(())
}

/// Where the headers and the data sit in each PEB of an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PebLayout {
    pub peb_size: u32,
    pub vid_header_offset: u32,
    pub data_offset: u32,
}

impl PebLayout {
    /// The bytes of a PEB that hold one LEB. Zero when the offsets are
    /// nonsense; [`Self::validate`] says why.
    pub fn leb_size(&self) -> u32 {
        self.peb_size.saturating_sub(self.data_offset)
    }

    /// The records in the volume table: one per possible volume id, as many
    /// as fit a LEB, at most [`MAX_VOLUMES`].
    pub fn vtbl_records(&self) -> u32 {
        MAX_VOLUMES.min(self.leb_size() / VTBL_RECORD_SIZE as u32)
    }

    /// The VID header must sit past the EC header and before the data, and
    /// the data must start inside the PEB.
    pub fn validate(&self) -> Result<(), HeaderError> {
        let header = HEADER_SIZE as u64;
        let vid_end = u64::from(self.vid_header_offset) + header;
        if u64::from(self.vid_header_offset) < header
            || vid_end > u64::from(self.data_offset)
            || self.data_offset >= self.peb_size
        {
            return Err(field(LAYOUT, "VID header offset", self.vid_header_offset));
        }
        Ok(())
    }
}

/// The erase-counter header at offset 0 of every PEB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcHeader {
    /// Gaia writes 0, as `ubinize` does.
    pub erase_counter: u64,
    pub vid_header_offset: u32,
    pub data_offset: u32,
    pub image_seq: u32,
}

impl EcHeader {
    /// `Ok(None)` for erased flash.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>, HeaderError> {
        if bytes.len() >= HEADER_SIZE && erased(bytes) {
            return Ok(None);
        }
        check(bytes, EC_MAGIC, EC)?;
        Ok(Some(Self {
            erase_counter: be64(bytes, 8),
            vid_header_offset: be32(bytes, 16),
            data_offset: be32(bytes, 20),
            image_seq: be32(bytes, 24),
        }))
    }

    /// The kernel's `validate_ec_hdr`, for the fields a file can check.
    pub fn validate(&self) -> Result<(), HeaderError> {
        if self.erase_counter > MAX_ERASE_COUNTER {
            return Err(field(EC, "erase counter", self.erase_counter));
        }
        if self.vid_header_offset < HEADER_SIZE as u32 || !self.vid_header_offset.is_multiple_of(8)
        {
            return Err(field(EC, "VID header offset", self.vid_header_offset));
        }
        if u64::from(self.data_offset) < u64::from(self.vid_header_offset) + HEADER_SIZE as u64 {
            return Err(field(EC, "data offset", self.data_offset));
        }
        Ok(())
    }
}

/// The volume-ID header of a PEB holding a LEB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VidHeader {
    pub kind: VolumeKind,
    /// 1 when the LEB was copied by wear levelling; Gaia writes 0.
    pub copy_flag: u8,
    /// The layout volume's is [`LAYOUT_VOLUME_COMPAT`]; other volumes' 0.
    pub compat: u8,
    pub volume_id: u32,
    pub lnum: u32,
    /// Static volumes only: the bytes in this LEB, the volume's LEB count
    /// and the CRC of this LEB's bytes. Zero for dynamic volumes.
    pub data_size: u32,
    pub used_ebs: u32,
    /// Gaia writes 0: every volume has alignment 1.
    pub data_pad: u32,
    pub data_crc: u32,
    /// Gaia writes 0: the kernel numbers writes from the highest it finds.
    pub sqnum: u64,
}

impl VidHeader {
    /// `Ok(None)` for erased flash: a PEB holding no LEB.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>, HeaderError> {
        if bytes.len() >= HEADER_SIZE && erased(bytes) {
            return Ok(None);
        }
        check(bytes, VID_MAGIC, VID)?;
        let kind = VolumeKind::from_code(bytes[5]).ok_or(HeaderError::VolumeType {
            what: VID,
            code: bytes[5],
        })?;
        Ok(Some(Self {
            kind,
            copy_flag: bytes[6],
            compat: bytes[7],
            volume_id: be32(bytes, 8),
            lnum: be32(bytes, 12),
            data_size: be32(bytes, 20),
            used_ebs: be32(bytes, 24),
            data_pad: be32(bytes, 28),
            data_crc: be32(bytes, 32),
            sqnum: be64(bytes, 40),
        }))
    }

    /// The kernel's `validate_vid_hdr`, less the checks that need the
    /// device's LEB size.
    pub fn validate(&self) -> Result<(), HeaderError> {
        if !matches!(self.compat, 0 | 1 | 2 | 4 | 5) {
            return Err(field(VID, "compat", self.compat));
        }
        if self.copy_flag > 1 {
            return Err(field(VID, "copy flag", self.copy_flag));
        }
        if self.volume_id >= MAX_VOLUMES && self.volume_id != LAYOUT_VOLUME_ID {
            return Err(field(VID, "volume id", self.volume_id));
        }
        if self.volume_id == LAYOUT_VOLUME_ID && self.compat != LAYOUT_VOLUME_COMPAT {
            return Err(field(VID, "layout volume compat", self.compat));
        }
        if self.volume_id < MAX_VOLUMES && self.compat != 0 {
            return Err(field(VID, "compat", self.compat));
        }
        if self.lnum > i32::MAX as u32 {
            return Err(field(VID, "LEB number", self.lnum));
        }
        match self.kind {
            VolumeKind::Static => {
                if self.used_ebs == 0 || self.lnum >= self.used_ebs {
                    return Err(field(VID, "used LEBs", self.used_ebs));
                }
                if self.data_size == 0 {
                    return Err(field(VID, "data size", self.data_size));
                }
            }
            VolumeKind::Dynamic => {
                if self.used_ebs != 0 {
                    return Err(field(VID, "used LEBs", self.used_ebs));
                }
                if self.copy_flag == 0 && self.data_crc != 0 {
                    return Err(field(VID, "data CRC", self.data_crc));
                }
                if self.copy_flag == 0 && self.data_size != 0 {
                    return Err(field(VID, "data size", self.data_size));
                }
            }
        }
        Ok(())
    }
}

/// One volume-table record that names a volume. A record whose
/// `reserved_pebs` is 0 is empty: no volume has its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtblRecord {
    pub reserved_pebs: u32,
    /// Gaia writes 1.
    pub alignment: u32,
    pub data_pad: u32,
    pub kind: VolumeKind,
    /// Set while an update writes the volume, cleared when it is done.
    pub upd_marker: bool,
    pub name: String,
    pub autoresize: bool,
}

impl VtblRecord {
    /// `Ok(None)` for an empty record.
    pub fn decode(record: &[u8]) -> Result<Option<Self>, HeaderError> {
        if record.len() < VTBL_RECORD_SIZE {
            return Err(HeaderError::TooShort {
                what: VTBL,
                len: record.len(),
            });
        }
        let computed = ubi_crc(&record[..VTBL_RECORD_CRC_SIZE]);
        let stored = be32(record, VTBL_RECORD_CRC_SIZE);
        if computed != stored {
            return Err(HeaderError::Crc {
                what: VTBL,
                stored,
                computed,
            });
        }
        let reserved_pebs = be32(record, 0);
        if reserved_pebs == 0 {
            return Ok(None);
        }
        let kind = VolumeKind::from_code(record[12]).ok_or(HeaderError::VolumeType {
            what: VTBL,
            code: record[12],
        })?;
        let name_len = usize::from(u16::from_be_bytes([record[14], record[15]]));
        if name_len == 0 || name_len > VOLUME_NAME_MAX {
            return Err(HeaderError::NameLength(name_len));
        }
        let name = std::str::from_utf8(&record[16..16 + name_len])
            .map_err(|_| HeaderError::NameNotUtf8)?
            .to_string();
        Ok(Some(Self {
            reserved_pebs,
            alignment: be32(record, 4),
            data_pad: be32(record, 8),
            kind,
            upd_marker: record[13] != 0,
            name,
            autoresize: record[144] & VTBL_AUTORESIZE != 0,
        }))
    }
}
