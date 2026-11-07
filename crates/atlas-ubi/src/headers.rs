//! The UBI header types under the names atlas-driver-soc and atlas-socflash
//! use. The codec is `gaia_formats::ubi`'s; these are the calling
//! conventions those crates were written against: `EcHeader::read` says
//! erased, header or corrupt in one value, and `VtblRecord::encode` takes an
//! `Option` (`None` for an empty record). New code names `gaia_formats::ubi`
//! directly.

use gaia_formats::ubi as gaia;

pub use gaia::{HEADER_SIZE, LAYOUT_VOLUME_ID, VTBL_RECORD_SIZE, VidHeader, VolumeKind};

/// What a header slot holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read<T> {
    /// Erased flash: every byte 0xff.
    Erased,
    Header(T),
    /// Neither: a wrong magic, version, CRC or field.
    Corrupt(String),
}

/// The erase-counter header at the start of a PEB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcHeader {
    pub erase_counter: u64,
    pub vid_header_offset: u32,
    pub data_offset: u32,
    pub image_seq: u32,
}

impl EcHeader {
    pub fn read(bytes: &[u8]) -> Read<Self> {
        match gaia::EcHeader::decode(bytes) {
            Ok(Some(ec)) => Read::Header(ec.into()),
            Ok(None) => Read::Erased,
            Err(error) => Read::Corrupt(error.to_string()),
        }
    }

    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        gaia::EcHeader::from(*self).encode()
    }
}

impl From<gaia::EcHeader> for EcHeader {
    fn from(ec: gaia::EcHeader) -> Self {
        Self {
            erase_counter: ec.erase_counter,
            vid_header_offset: ec.vid_header_offset,
            data_offset: ec.data_offset,
            image_seq: ec.image_seq,
        }
    }
}

impl From<EcHeader> for gaia::EcHeader {
    fn from(ec: EcHeader) -> Self {
        Self {
            erase_counter: ec.erase_counter,
            vid_header_offset: ec.vid_header_offset,
            data_offset: ec.data_offset,
            image_seq: ec.image_seq,
        }
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
    /// The record's bytes; `None` encodes an empty record. Panics on a record
    /// gaia's validation refuses (an empty or over-long name, no reservation):
    /// the kernel would refuse it too.
    pub fn encode(record: Option<&Self>) -> [u8; VTBL_RECORD_SIZE] {
        match record {
            None => gaia::VtblRecord::empty(),
            Some(record) => gaia::VtblRecord::from(record)
                .encode()
                .unwrap_or_else(|error| panic!("volume table record {}: {error}", record.name)),
        }
    }
}

impl From<&VtblRecord> for gaia::VtblRecord {
    fn from(record: &VtblRecord) -> Self {
        Self {
            reserved_pebs: record.reserved_pebs,
            alignment: record.alignment,
            data_pad: record.data_pad,
            kind: record.kind,
            upd_marker: record.upd_marker,
            name: record.name.clone(),
            autoresize: record.autoresize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erased_header_and_empty_record() {
        assert_eq!(EcHeader::read(&[0xff; HEADER_SIZE]), Read::Erased);
        assert_eq!(VtblRecord::encode(None), gaia::VtblRecord::empty());
        let ec = EcHeader {
            erase_counter: 7,
            vid_header_offset: 512,
            data_offset: 1024,
            image_seq: 9,
        };
        assert_eq!(EcHeader::read(&ec.encode()), Read::Header(ec));
    }
}
