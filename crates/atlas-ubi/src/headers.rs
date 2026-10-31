//! UBI's on-flash headers (`drivers/mtd/ubi/ubi-media.h`), big-endian, each
//! closed by its CRC: the erase-counter (EC) header at the start of every
//! PEB, the volume-ID (VID) header of every PEB holding data, and the
//! volume-table records the layout volume holds.

use crate::crc::ubi_crc;

pub const EC_MAGIC: u32 = 0x5542_4923; // "UBI#"
pub const VID_MAGIC: u32 = 0x5542_4921; // "UBI!"
pub const VERSION: u8 = 1;
pub const HEADER_SIZE: usize = 64;
const HEADER_CRC_SIZE: usize = 60;
pub const VTBL_RECORD_SIZE: usize = 172;
const VTBL_RECORD_CRC_SIZE: usize = VTBL_RECORD_SIZE - 4;
pub const VTBL_MAX_RECORDS: usize = 128;
pub const VOLUME_NAME_MAX: usize = 127;
pub const LAYOUT_VOLUME_ID: u32 = 0x7fff_efff;
pub const LAYOUT_VOLUME_LEBS: u32 = 2;
pub const VTBL_AUTORESIZE: u8 = 0x01;
pub const ERASED: u8 = 0xff;

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn be64(bytes: &[u8], at: usize) -> u64 {
    (u64::from(be32(bytes, at)) << 32) | u64::from(be32(bytes, at + 4))
}

/// What a header slot holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read<T> {
    /// Erased flash: every byte 0xff.
    Erased,
    Header(T),
    /// Neither: a wrong magic, version or CRC.
    Corrupt(String),
}

fn check(bytes: &[u8], magic: u32, what: &str) -> Option<String> {
    if bytes.len() < HEADER_SIZE {
        return Some(format!("{what} header cut short"));
    }
    let found = be32(bytes, 0);
    if found != magic {
        return Some(format!("{what} header magic {found:#010x}"));
    }
    if bytes[4] != VERSION {
        return Some(format!("{what} header version {}", bytes[4]));
    }
    let crc = ubi_crc(&bytes[..HEADER_CRC_SIZE]);
    let stored = be32(bytes, HEADER_CRC_SIZE);
    (crc != stored).then(|| format!("{what} header CRC {stored:#010x}, computed {crc:#010x}"))
}

fn erased(bytes: &[u8]) -> bool {
    bytes.iter().take(HEADER_SIZE).all(|byte| *byte == ERASED)
}

fn seal(header: &mut [u8; HEADER_SIZE]) {
    let crc = ubi_crc(&header[..HEADER_CRC_SIZE]);
    header[HEADER_CRC_SIZE..].copy_from_slice(&crc.to_be_bytes());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcHeader {
    pub erase_counter: u64,
    pub vid_header_offset: u32,
    pub data_offset: u32,
    pub image_seq: u32,
}

impl EcHeader {
    pub fn read(bytes: &[u8]) -> Read<Self> {
        if erased(bytes) {
            return Read::Erased;
        }
        if let Some(problem) = check(bytes, EC_MAGIC, "EC") {
            return Read::Corrupt(problem);
        }
        Read::Header(Self {
            erase_counter: be64(bytes, 8),
            vid_header_offset: be32(bytes, 16),
            data_offset: be32(bytes, 20),
            image_seq: be32(bytes, 24),
        })
    }

    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&EC_MAGIC.to_be_bytes());
        header[4] = VERSION;
        header[8..16].copy_from_slice(&self.erase_counter.to_be_bytes());
        header[16..20].copy_from_slice(&self.vid_header_offset.to_be_bytes());
        header[20..24].copy_from_slice(&self.data_offset.to_be_bytes());
        header[24..28].copy_from_slice(&self.image_seq.to_be_bytes());
        seal(&mut header);
        header
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeKind {
    Dynamic,
    Static,
}

impl VolumeKind {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Dynamic),
            2 => Some(Self::Static),
            _ => None,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Dynamic => 1,
            Self::Static => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VidHeader {
    pub kind: VolumeKind,
    pub copy_flag: u8,
    pub compat: u8,
    pub volume_id: u32,
    pub lnum: u32,
    /// Static volumes: the bytes in this LEB, the volume's LEB count and the
    /// CRC of this LEB's bytes. 0 for dynamic ones.
    pub data_size: u32,
    pub used_ebs: u32,
    pub data_pad: u32,
    pub data_crc: u32,
    pub sqnum: u64,
}

impl VidHeader {
    pub fn read(bytes: &[u8]) -> Read<Self> {
        if erased(bytes) {
            return Read::Erased;
        }
        if let Some(problem) = check(bytes, VID_MAGIC, "VID") {
            return Read::Corrupt(problem);
        }
        let Some(kind) = VolumeKind::from_code(bytes[5]) else {
            return Read::Corrupt(format!("VID header volume type {}", bytes[5]));
        };
        Read::Header(Self {
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
        })
    }

    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&VID_MAGIC.to_be_bytes());
        header[4] = VERSION;
        header[5] = self.kind.code();
        header[6] = self.copy_flag;
        header[7] = self.compat;
        header[8..12].copy_from_slice(&self.volume_id.to_be_bytes());
        header[12..16].copy_from_slice(&self.lnum.to_be_bytes());
        header[20..24].copy_from_slice(&self.data_size.to_be_bytes());
        header[24..28].copy_from_slice(&self.used_ebs.to_be_bytes());
        header[28..32].copy_from_slice(&self.data_pad.to_be_bytes());
        header[32..36].copy_from_slice(&self.data_crc.to_be_bytes());
        header[40..48].copy_from_slice(&self.sqnum.to_be_bytes());
        seal(&mut header);
        header
    }
}

/// One volume-table record that names a volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtblRecord {
    pub reserved_pebs: u32,
    pub alignment: u32,
    pub data_pad: u32,
    pub kind: VolumeKind,
    pub upd_marker: bool,
    pub name: String,
    pub autoresize: bool,
}

impl VtblRecord {
    /// `Ok(None)` for an empty record (no volume with this id).
    pub fn read(record: &[u8]) -> Result<Option<Self>, String> {
        let crc = ubi_crc(&record[..VTBL_RECORD_CRC_SIZE]);
        let stored = be32(record, VTBL_RECORD_CRC_SIZE);
        if crc != stored {
            return Err(format!(
                "volume table record CRC {stored:#010x}, computed {crc:#010x}"
            ));
        }
        let reserved_pebs = be32(record, 0);
        if reserved_pebs == 0 {
            return Ok(None);
        }
        let kind = VolumeKind::from_code(record[12])
            .ok_or_else(|| format!("volume table record type {}", record[12]))?;
        let name_len = usize::from(u16::from_be_bytes([record[14], record[15]]));
        if name_len == 0 || name_len > VOLUME_NAME_MAX {
            return Err(format!("volume table record name length {name_len}"));
        }
        let name = std::str::from_utf8(&record[16..16 + name_len])
            .map_err(|_| "volume table record name isn't UTF-8".to_string())?
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

    /// The record's bytes; `None` encodes an empty record.
    pub fn encode(record: Option<&Self>) -> [u8; VTBL_RECORD_SIZE] {
        let mut bytes = [0u8; VTBL_RECORD_SIZE];
        if let Some(record) = record {
            bytes[0..4].copy_from_slice(&record.reserved_pebs.to_be_bytes());
            bytes[4..8].copy_from_slice(&record.alignment.to_be_bytes());
            bytes[8..12].copy_from_slice(&record.data_pad.to_be_bytes());
            bytes[12] = record.kind.code();
            bytes[13] = u8::from(record.upd_marker);
            bytes[14..16].copy_from_slice(&(record.name.len() as u16).to_be_bytes());
            bytes[16..16 + record.name.len()].copy_from_slice(record.name.as_bytes());
            bytes[144] = if record.autoresize {
                VTBL_AUTORESIZE
            } else {
                0
            };
        }
        let crc = ubi_crc(&bytes[..VTBL_RECORD_CRC_SIZE]);
        bytes[VTBL_RECORD_CRC_SIZE..].copy_from_slice(&crc.to_be_bytes());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_round_trip_and_reject_damage() {
        let ec = EcHeader {
            erase_counter: 7,
            vid_header_offset: 2048,
            data_offset: 4096,
            image_seq: 0x1234_5678,
        };
        let mut bytes = ec.encode();
        assert_eq!(EcHeader::read(&bytes), Read::Header(ec));
        bytes[9] ^= 1;
        assert!(matches!(EcHeader::read(&bytes), Read::Corrupt(p) if p.contains("CRC")));
        assert_eq!(EcHeader::read(&[0xff; 64]), Read::Erased);

        let vid = VidHeader {
            kind: VolumeKind::Static,
            copy_flag: 0,
            compat: 0,
            volume_id: 3,
            lnum: 9,
            data_size: 100,
            used_ebs: 10,
            data_pad: 0,
            data_crc: 0xdead_beef,
            sqnum: 0,
        };
        assert_eq!(VidHeader::read(&vid.encode()), Read::Header(vid));
        assert!(matches!(
            VidHeader::read(&ec.encode()),
            Read::Corrupt(p) if p.contains("magic")
        ));

        let record = VtblRecord {
            reserved_pebs: 40,
            alignment: 1,
            data_pad: 0,
            kind: VolumeKind::Dynamic,
            upd_marker: false,
            name: "rootfs_a".into(),
            autoresize: false,
        };
        assert_eq!(
            VtblRecord::read(&VtblRecord::encode(Some(&record))),
            Ok(Some(record))
        );
        let empty = VtblRecord::encode(None);
        assert_eq!(VtblRecord::read(&empty), Ok(None));
        // Gaia writes empty records with this CRC, as ubinize does.
        assert_eq!(be32(&empty, 168), 0xf116_c36b);
    }
}
