//! Reading a UBI image (Gaia's `[[image.assembly.ubis]]`, or ubinize's):
//! every PEB's headers checked, the layout volume's two tables compared,
//! and each volume's reservation and the LEBs the image carries for it.

use std::collections::BTreeMap;
use std::io::{Read as IoRead, Seek, SeekFrom};

use crate::crc::ubi_crc;
use crate::headers::{
    ERASED, EcHeader, HEADER_SIZE, LAYOUT_VOLUME_ID, LAYOUT_VOLUME_LEBS, Read, VTBL_MAX_RECORDS,
    VTBL_RECORD_SIZE, VidHeader, VolumeKind, VtblRecord,
};

/// Where the headers and data sit in each PEB. The kernel derives the
/// offsets from the flash's page and sub-page sizes and refuses an image
/// whose offsets differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub peb_size: u32,
    pub vid_header_offset: u32,
    pub data_offset: u32,
}

impl Geometry {
    pub fn leb_size(&self) -> u32 {
        self.peb_size - self.data_offset
    }

    /// Volume-table records in the layout volume: one per possible volume,
    /// as many as fit a LEB, at most 128.
    pub fn vtbl_records(&self) -> usize {
        (self.leb_size() as usize / VTBL_RECORD_SIZE).min(VTBL_MAX_RECORDS)
    }
}

/// A volume the image declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub id: u32,
    pub name: String,
    pub kind: VolumeKind,
    pub reserved_lebs: u32,
    pub autoresize: bool,
    /// The LEBs the image carries (0 for a volume the system fills).
    pub image_lebs: u32,
    /// Static volumes: the data bytes, from the VID headers.
    pub static_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub geometry: Geometry,
    pub image_seq: u32,
    /// PEBs in the image.
    pub pebs: u32,
    pub volumes: Vec<Volume>,
}

impl Image {
    pub fn volume(&self, name: &str) -> Option<&Volume> {
        self.volumes.iter().find(|volume| volume.name == name)
    }

    /// The PEBs a flash needs for this image, before bad-block reserves:
    /// the layout volume, UBI's two working PEBs and every reservation.
    pub fn reserved_pebs(&self) -> u64 {
        u64::from(LAYOUT_VOLUME_LEBS)
            + 2
            + self
                .volumes
                .iter()
                .map(|volume| u64::from(volume.reserved_lebs))
                .sum::<u64>()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UbiError(pub String);

impl std::fmt::Display for UbiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UbiError {}

fn fail<T>(message: impl Into<String>) -> Result<T, UbiError> {
    Err(UbiError(message.into()))
}

/// The eraseblock size of an image: the distance to the second PEB's EC
/// header, tried at every power of two from 4 KiB to 4 MiB.
pub fn find_peb_size(image: &mut (impl IoRead + Seek)) -> Result<u32, UbiError> {
    let length = image
        .seek(SeekFrom::End(0))
        .map_err(|error| UbiError(format!("the image: {error}")))?;
    let mut header = [0u8; HEADER_SIZE];
    for shift in 12..=22u32 {
        let size = 1u64 << shift;
        if size >= length {
            break;
        }
        image
            .seek(SeekFrom::Start(size))
            .and_then(|_| image.read_exact(&mut header))
            .map_err(|error| UbiError(format!("the image: {error}")))?;
        if matches!(EcHeader::read(&header), Read::Header(_)) {
            return Ok(size as u32);
        }
    }
    // A one-PEB image has no second header; it's not a usable image anyway.
    fail(
        "no second erase-counter header at any power-of-two offset: not a UBI image of two or more PEBs",
    )
}

/// Reads and checks a whole image of `peb_size` eraseblocks.
pub fn read_image(image: &mut (impl IoRead + Seek), peb_size: u32) -> Result<Image, UbiError> {
    let length = image
        .seek(SeekFrom::End(0))
        .map_err(|error| UbiError(format!("the image: {error}")))?;
    image
        .rewind()
        .map_err(|error| UbiError(format!("the image: {error}")))?;
    if peb_size == 0 || length == 0 || length % u64::from(peb_size) != 0 {
        return fail(format!(
            "the image is {length} bytes, not a whole number of {peb_size}-byte PEBs"
        ));
    }
    let pebs = u32::try_from(length / u64::from(peb_size))
        .map_err(|_| UbiError("the image has too many PEBs".into()))?;
    if pebs < LAYOUT_VOLUME_LEBS {
        return fail("the image is shorter than its layout volume");
    }

    let mut geometry: Option<Geometry> = None;
    let mut image_seq = 0;
    let mut tables: Vec<Vec<u8>> = Vec::new();
    // Per volume id: the LEBs seen, and static volumes' byte totals.
    let mut lebs: BTreeMap<u32, (u32, u64, VolumeKind)> = BTreeMap::new();
    let mut peb = vec![0u8; peb_size as usize];
    for index in 0..pebs {
        image
            .read_exact(&mut peb)
            .map_err(|error| UbiError(format!("PEB {index}: {error}")))?;
        let ec = match EcHeader::read(&peb) {
            Read::Header(ec) => ec,
            Read::Erased => return fail(format!("PEB {index} is erased: an image has none")),
            Read::Corrupt(problem) => return fail(format!("PEB {index}: {problem}")),
        };
        let this = Geometry {
            peb_size,
            vid_header_offset: ec.vid_header_offset,
            data_offset: ec.data_offset,
        };
        match geometry {
            None => {
                if this.vid_header_offset as usize + HEADER_SIZE > this.data_offset as usize
                    || this.data_offset >= peb_size
                {
                    return fail(format!(
                        "PEB {index}: VID header at {}, data at {}, in a {peb_size}-byte PEB",
                        this.vid_header_offset, this.data_offset
                    ));
                }
                geometry = Some(this);
                image_seq = ec.image_seq;
            }
            Some(first) if first != this || ec.image_seq != image_seq => {
                return fail(format!(
                    "PEB {index}: its offsets or image sequence differ from PEB 0's"
                ));
            }
            Some(_) => {}
        }
        let vid_at = this.vid_header_offset as usize;
        let vid = match VidHeader::read(&peb[vid_at..vid_at + HEADER_SIZE]) {
            Read::Header(vid) => vid,
            Read::Erased => return fail(format!("PEB {index} holds no volume")),
            Read::Corrupt(problem) => return fail(format!("PEB {index}: {problem}")),
        };
        let data = &peb[this.data_offset as usize..];
        if vid.volume_id == LAYOUT_VOLUME_ID {
            if vid.lnum != tables.len() as u32 || index >= LAYOUT_VOLUME_LEBS {
                return fail(format!(
                    "PEB {index}: layout volume LEB {} out of place",
                    vid.lnum
                ));
            }
            tables.push(data[..this.vtbl_records() * VTBL_RECORD_SIZE].to_vec());
            continue;
        }
        let entry = lebs.entry(vid.volume_id).or_insert((0, 0, vid.kind));
        if vid.lnum != entry.0 {
            return fail(format!(
                "PEB {index}: volume {} LEB {} where LEB {} was expected",
                vid.volume_id, vid.lnum, entry.0
            ));
        }
        if vid.kind == VolumeKind::Static {
            let size = vid.data_size as usize;
            if size > data.len() || ubi_crc(&data[..size]) != vid.data_crc {
                return fail(format!(
                    "PEB {index}: volume {} LEB {}'s data doesn't match its CRC",
                    vid.volume_id, vid.lnum
                ));
            }
            entry.1 += size as u64;
        }
        entry.0 += 1;
    }

    let Some(geometry) = geometry else {
        return fail("the image is empty");
    };
    if tables.len() != LAYOUT_VOLUME_LEBS as usize {
        return fail("the image lacks the layout volume's two copies");
    }
    if tables[0] != tables[1] {
        return fail("the layout volume's two volume tables differ");
    }
    let mut volumes = Vec::new();
    for (id, record) in tables[0]
        .as_chunks::<VTBL_RECORD_SIZE>()
        .0
        .iter()
        .enumerate()
    {
        let record = VtblRecord::read(record)
            .map_err(|problem| UbiError(format!("volume table record {id}: {problem}")))?;
        let Some(record) = record else { continue };
        let id = id as u32;
        let (image_lebs, static_bytes, kind) = lebs.remove(&id).unwrap_or((0, 0, record.kind));
        if kind != record.kind {
            return fail(format!(
                "volume {id} ({}) is {:?} in the table but its LEBs say {kind:?}",
                record.name, record.kind
            ));
        }
        if image_lebs > record.reserved_pebs {
            return fail(format!(
                "volume {id} ({}) carries {image_lebs} LEBs but reserves {}",
                record.name, record.reserved_pebs
            ));
        }
        volumes.push(Volume {
            id,
            name: record.name,
            kind: record.kind,
            reserved_lebs: record.reserved_pebs,
            autoresize: record.autoresize,
            image_lebs,
            static_bytes: (record.kind == VolumeKind::Static).then_some(static_bytes),
        });
    }
    if let Some(id) = lebs.keys().next() {
        return fail(format!(
            "PEBs belong to volume {id}, which the table doesn't list"
        ));
    }
    Ok(Image {
        geometry,
        image_seq,
        pebs,
        volumes,
    })
}

/// The bytes of `peb` worth programming: up to its last non-erased byte,
/// rounded up to whole pages (ubiformat's `drop_ffs`). Programming the
/// erased pages after it would leave them unusable for UBI's later writes.
pub fn programmed_len(peb: &[u8], page_size: usize) -> usize {
    let used = peb
        .iter()
        .rposition(|byte| *byte != ERASED)
        .map_or(0, |at| at + 1);
    used.div_ceil(page_size) * page_size
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::headers::{EcHeader, VidHeader, VtblRecord};

    pub const SEQ: u32 = 0x1234_5678;

    /// A small image as Gaia writes it (its ubi.rs, field for field): 16 KiB
    /// PEBs, 512-byte pages, rootfs_a with data, rootfs_b and data empty.
    pub fn sample(rootfs: &[u8]) -> (Geometry, Vec<u8>) {
        let geometry = Geometry {
            peb_size: 16 * 1024,
            vid_header_offset: 512,
            data_offset: 1024,
        };
        let leb = geometry.leb_size() as usize;
        let mut table = Vec::new();
        let names = [
            ("rootfs_a", 4, false),
            ("rootfs_b", 4, false),
            ("data", 1, true),
        ];
        for id in 0..geometry.vtbl_records() {
            let record = names
                .get(id)
                .map(|(name, reserved, autoresize)| VtblRecord {
                    reserved_pebs: *reserved,
                    alignment: 1,
                    data_pad: 0,
                    kind: VolumeKind::Dynamic,
                    upd_marker: false,
                    name: (*name).into(),
                    autoresize: *autoresize,
                });
            table.extend_from_slice(&VtblRecord::encode(record.as_ref()));
        }
        let peb = |volume_id: u32, lnum: u32, compat: u8, data: &[u8]| {
            let mut peb = vec![0xffu8; geometry.peb_size as usize];
            let ec = EcHeader {
                erase_counter: 0,
                vid_header_offset: geometry.vid_header_offset,
                data_offset: geometry.data_offset,
                image_seq: SEQ,
            };
            peb[..64].copy_from_slice(&ec.encode());
            let vid = VidHeader {
                kind: VolumeKind::Dynamic,
                copy_flag: 0,
                compat,
                volume_id,
                lnum,
                data_size: 0,
                used_ebs: 0,
                data_pad: 0,
                data_crc: 0,
                sqnum: 0,
            };
            peb[512..576].copy_from_slice(&vid.encode());
            peb[1024..1024 + data.len()].copy_from_slice(data);
            peb
        };
        let mut image = Vec::new();
        image.extend(peb(LAYOUT_VOLUME_ID, 0, 5, &table));
        image.extend(peb(LAYOUT_VOLUME_ID, 1, 5, &table));
        for (lnum, chunk) in rootfs.chunks(leb).enumerate() {
            image.extend(peb(0, lnum as u32, 0, chunk));
        }
        (geometry, image)
    }

    #[test]
    fn a_gaia_style_image_reads_back() {
        let rootfs: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let (geometry, bytes) = sample(&rootfs);
        let mut cursor = std::io::Cursor::new(&bytes);
        assert_eq!(find_peb_size(&mut cursor), Ok(16 * 1024));
        let image = read_image(&mut cursor, 16 * 1024).unwrap();
        assert_eq!(image.geometry, geometry);
        assert_eq!((image.pebs, image.image_seq), (4, SEQ));
        let names: Vec<(&str, u32, u32)> = image
            .volumes
            .iter()
            .map(|v| (v.name.as_str(), v.reserved_lebs, v.image_lebs))
            .collect();
        assert_eq!(
            names,
            [("rootfs_a", 4, 2), ("rootfs_b", 4, 0), ("data", 1, 0)]
        );
        assert!(image.volume("data").unwrap().autoresize);
        assert_eq!(image.reserved_pebs(), 2 + 2 + 9);
    }

    #[test]
    fn damage_is_named() {
        let (_, mut bytes) = sample(&[1; 100]);
        bytes[16 * 1024 + 4096] ^= 1; // the second table copy
        let error = read_image(&mut std::io::Cursor::new(&bytes), 16 * 1024).unwrap_err();
        assert!(error.0.contains("differ"), "{error}");
        let (_, mut bytes) = sample(&[1; 100]);
        bytes[2 * 16 * 1024 + 512 + 13] ^= 1; // a VID header's lnum
        let error = read_image(&mut std::io::Cursor::new(&bytes), 16 * 1024).unwrap_err();
        assert!(error.0.contains("CRC"), "{error}");
        let error = read_image(&mut std::io::Cursor::new(&bytes[..1000]), 16 * 1024).unwrap_err();
        assert!(error.0.contains("whole number"), "{error}");
    }

    #[test]
    fn trailing_erased_pages_are_not_programmed() {
        let mut peb = vec![0xffu8; 8 * 512];
        assert_eq!(programmed_len(&peb, 512), 0);
        peb[0] = 0;
        assert_eq!(programmed_len(&peb, 512), 512);
        peb[1024 + 7] = 0x12;
        assert_eq!(programmed_len(&peb, 512), 1536);
    }
}
