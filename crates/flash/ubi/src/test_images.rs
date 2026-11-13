//! Test images laid out the way the image builder lays them out, built here
//! with test-only encoders (the writer is not part of this crate) and read
//! back by the reader. Every image is generated in code; none is stored.

use crate::crc::ubi_crc;
use crate::headers::{
    EC_MAGIC, ERASED, EcHeader, HEADER_SIZE, HeaderError, LAYOUT_VOLUME_COMPAT, LAYOUT_VOLUME_ID,
    PebLayout, VERSION, VID_MAGIC, VTBL_RECORD_SIZE, VidHeader, VolumeKind, VtblRecord,
};
use crate::image::{ImageError, find_peb_size, read_image};
use std::io::Cursor;

pub const SEQ: u32 = 0x1234_5678;
pub const PEB: usize = 16 * 1024;

/// Writes the CRC over every byte of `bytes` but the last four.
fn seal(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let crc = ubi_crc(&bytes[..end]);
    bytes[end..].copy_from_slice(&crc.to_be_bytes());
}

pub fn enc_ec(ec: &EcHeader) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];
    header[0..4].copy_from_slice(&EC_MAGIC.to_be_bytes());
    header[4] = VERSION;
    header[8..16].copy_from_slice(&ec.erase_counter.to_be_bytes());
    header[16..20].copy_from_slice(&ec.vid_header_offset.to_be_bytes());
    header[20..24].copy_from_slice(&ec.data_offset.to_be_bytes());
    header[24..28].copy_from_slice(&ec.image_seq.to_be_bytes());
    seal(&mut header);
    header
}

pub fn enc_vid(vid: &VidHeader) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];
    header[0..4].copy_from_slice(&VID_MAGIC.to_be_bytes());
    header[4] = VERSION;
    header[5] = vid.kind.code();
    header[6] = vid.copy_flag;
    header[7] = vid.compat;
    header[8..12].copy_from_slice(&vid.volume_id.to_be_bytes());
    header[12..16].copy_from_slice(&vid.lnum.to_be_bytes());
    header[20..24].copy_from_slice(&vid.data_size.to_be_bytes());
    header[24..28].copy_from_slice(&vid.used_ebs.to_be_bytes());
    header[28..32].copy_from_slice(&vid.data_pad.to_be_bytes());
    header[32..36].copy_from_slice(&vid.data_crc.to_be_bytes());
    header[40..48].copy_from_slice(&vid.sqnum.to_be_bytes());
    seal(&mut header);
    header
}

pub fn enc_record(record: &VtblRecord) -> [u8; VTBL_RECORD_SIZE] {
    assert!(
        !record.name.is_empty() && record.name.len() <= 127,
        "test name"
    );
    let mut bytes = [0u8; VTBL_RECORD_SIZE];
    bytes[0..4].copy_from_slice(&record.reserved_pebs.to_be_bytes());
    bytes[4..8].copy_from_slice(&record.alignment.to_be_bytes());
    bytes[8..12].copy_from_slice(&record.data_pad.to_be_bytes());
    bytes[12] = record.kind.code();
    bytes[13] = u8::from(record.upd_marker);
    bytes[14..16].copy_from_slice(&(record.name.len() as u16).to_be_bytes());
    bytes[16..16 + record.name.len()].copy_from_slice(record.name.as_bytes());
    bytes[144] = if record.autoresize { 1 } else { 0 };
    seal(&mut bytes);
    bytes
}

/// An empty record: zeros and their CRC.
pub fn enc_empty_record() -> [u8; VTBL_RECORD_SIZE] {
    let mut bytes = [0u8; VTBL_RECORD_SIZE];
    seal(&mut bytes);
    bytes
}

/// One whole PEB: the EC header, the VID header and the LEB's bytes, with
/// erased bytes between them and after the data.
pub fn encode_peb(layout: &PebLayout, ec: &EcHeader, vid: &VidHeader, data: &[u8]) -> Vec<u8> {
    let mut peb = vec![ERASED; layout.peb_size as usize];
    peb[..HEADER_SIZE].copy_from_slice(&enc_ec(ec));
    let vid_at = layout.vid_header_offset as usize;
    peb[vid_at..vid_at + HEADER_SIZE].copy_from_slice(&enc_vid(vid));
    let data_at = layout.data_offset as usize;
    peb[data_at..data_at + data.len()].copy_from_slice(data);
    peb
}

/// The volume table a layout-volume LEB holds: each volume's record at its id,
/// every other record empty.
pub fn encode_volume_table(layout: &PebLayout, volumes: &[(u32, VtblRecord)]) -> Vec<u8> {
    let records = layout.vtbl_records() as usize;
    let mut table = Vec::with_capacity(records * VTBL_RECORD_SIZE);
    for _ in 0..records {
        table.extend_from_slice(&enc_empty_record());
    }
    for (id, record) in volumes {
        let at = *id as usize;
        assert!(at < records, "volume id {id} past the table");
        table[at * VTBL_RECORD_SIZE..][..VTBL_RECORD_SIZE].copy_from_slice(&enc_record(record));
    }
    table
}

pub fn layout16k() -> PebLayout {
    PebLayout {
        peb_size: PEB as u32,
        vid_header_offset: 512,
        data_offset: 1024,
    }
}

pub fn ec_for(layout: &PebLayout, seq: u32) -> EcHeader {
    EcHeader {
        erase_counter: 0,
        vid_header_offset: layout.vid_header_offset,
        data_offset: layout.data_offset,
        image_seq: seq,
    }
}

pub fn vid(kind: VolumeKind, volume_id: u32, lnum: u32) -> VidHeader {
    VidHeader {
        kind,
        copy_flag: 0,
        compat: 0,
        volume_id,
        lnum,
        data_size: 0,
        used_ebs: 0,
        data_pad: 0,
        data_crc: 0,
        sqnum: 0,
    }
}

pub fn record(reserved: u32, kind: VolumeKind, name: &str, autoresize: bool) -> VtblRecord {
    VtblRecord {
        reserved_pebs: reserved,
        alignment: 1,
        data_pad: 0,
        kind,
        upd_marker: false,
        name: name.to_string(),
        autoresize,
    }
}

/// A small image: 16 KiB PEBs, 512-byte pages, the layout volume in PEBs 0
/// and 1, then rootfs_a's LEBs. rootfs_a, rootfs_b and data (autoresize);
/// rootfs_b and data carry no LEBs.
pub fn dev_sample(rootfs: &[u8]) -> Vec<u8> {
    let layout = layout16k();
    let leb = layout.leb_size() as usize;
    let table = encode_volume_table(
        &layout,
        &[
            (0, record(4, VolumeKind::Dynamic, "rootfs_a", false)),
            (1, record(4, VolumeKind::Dynamic, "rootfs_b", false)),
            (2, record(1, VolumeKind::Dynamic, "data", true)),
        ],
    );
    let ec = ec_for(&layout, SEQ);
    let layout_vid = |lnum| VidHeader {
        compat: LAYOUT_VOLUME_COMPAT,
        ..vid(VolumeKind::Dynamic, LAYOUT_VOLUME_ID, lnum)
    };
    let mut image = Vec::new();
    image.extend(encode_peb(&layout, &ec, &layout_vid(0), &table));
    image.extend(encode_peb(&layout, &ec, &layout_vid(1), &table));
    for (lnum, chunk) in rootfs.chunks(leb).enumerate() {
        image.extend(encode_peb(
            &layout,
            &ec,
            &vid(VolumeKind::Dynamic, 0, lnum as u32),
            chunk,
        ));
    }
    image
}

/// The sample Gaia's own tests use: rootfs_a (dynamic, data), rootfs_b
/// (empty), kernel (static, data) and data (autoresize, empty).
pub fn gaia_sample(rootfs: &[u8], kernel: &[u8]) -> Vec<u8> {
    let layout = layout16k();
    let leb = layout.leb_size() as usize;
    let table = encode_volume_table(
        &layout,
        &[
            (0, record(4, VolumeKind::Dynamic, "rootfs_a", false)),
            (1, record(4, VolumeKind::Dynamic, "rootfs_b", false)),
            (2, record(1, VolumeKind::Static, "kernel", false)),
            (3, record(1, VolumeKind::Dynamic, "data", true)),
        ],
    );
    let ec = ec_for(&layout, SEQ);
    let layout_vid = |lnum| VidHeader {
        compat: LAYOUT_VOLUME_COMPAT,
        ..vid(VolumeKind::Dynamic, LAYOUT_VOLUME_ID, lnum)
    };
    let mut image = Vec::new();
    image.extend(encode_peb(&layout, &ec, &layout_vid(0), &table));
    image.extend(encode_peb(&layout, &ec, &layout_vid(1), &table));
    for (lnum, chunk) in rootfs.chunks(leb).enumerate() {
        image.extend(encode_peb(
            &layout,
            &ec,
            &vid(VolumeKind::Dynamic, 0, lnum as u32),
            chunk,
        ));
    }
    let used = kernel.len().div_ceil(leb) as u32;
    for (lnum, chunk) in kernel.chunks(leb).enumerate() {
        let header = VidHeader {
            data_size: chunk.len() as u32,
            used_ebs: used,
            data_crc: ubi_crc(chunk),
            ..vid(VolumeKind::Static, 2, lnum as u32)
        };
        image.extend(encode_peb(&layout, &ec, &header, chunk));
    }
    image
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn rootfs(len: u32) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn a_small_image_reads_back() {
    let data = rootfs(20_000);
    let bytes = dev_sample(&data);
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(find_peb_size(&mut cursor).unwrap(), PEB as u32);
    let image = read_image(&mut cursor, PEB as u32).unwrap();
    assert_eq!(image.layout, layout16k());
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
    assert_eq!(image.volume("rootfs_a").unwrap().first_peb, Some(2));
    assert!(image.volume("data").unwrap().autoresize);
    assert_eq!(image.reserved_pebs(), 2 + 2 + 9);
    // The first LEB's bytes sit at the data offset of PEB 2.
    assert_eq!(&bytes[2 * PEB + 1024..][..15_360], &data[..15_360]);
}

#[test]
fn a_small_image_damage_is_named() {
    let read = |bytes: &[u8]| read_image(&mut Cursor::new(bytes), PEB as u32).unwrap_err();
    let mut bytes = dev_sample(&rootfs(100));
    bytes[PEB + 4096] ^= 1; // the second table copy
    let error = read(&bytes);
    assert!(error.to_string().contains("differ"), "{error}");
    assert!(matches!(error, ImageError::TablesDiffer));

    let mut bytes = dev_sample(&rootfs(100));
    bytes[2 * PEB + 512 + 13] ^= 1; // a VID header's lnum
    let error = read(&bytes);
    assert!(error.to_string().contains("CRC"), "{error}");

    let error = read(&dev_sample(&rootfs(100))[..1000]);
    assert!(error.to_string().contains("whole number"), "{error}");
}

#[test]
fn the_bytes_sit_where_ubi_media_h_puts_them() {
    let image = gaia_sample(&rootfs(20_000), &[0x5a; 100]);
    assert_eq!(image.len(), 5 * PEB);
    for peb in image.chunks(PEB) {
        assert_eq!(&peb[0..4], b"UBI#");
        assert_eq!(peb[4], 1);
        assert_eq!(&peb[8..16], &[0; 8], "erase counter");
        assert_eq!(be32(peb, 16), 512, "vid_hdr_offset");
        assert_eq!(be32(peb, 20), 1024, "data_offset");
        assert_eq!(be32(peb, 24), SEQ, "image_seq");
        assert_eq!(be32(peb, 60), ubi_crc(&peb[..60]), "hdr_crc");
        assert!(peb[64..512].iter().all(|byte| *byte == 0xff));
        let vid_hdr = &peb[512..576];
        assert_eq!(&vid_hdr[0..4], b"UBI!");
        assert_eq!(vid_hdr[4], 1);
        assert_eq!(vid_hdr[6], 0, "copy_flag");
        assert_eq!(&vid_hdr[40..48], &[0; 8], "sqnum");
        assert_eq!(be32(vid_hdr, 60), ubi_crc(&vid_hdr[..60]));
    }
    let layout_vid = &image[PEB + 512..][..64];
    assert_eq!((layout_vid[5], layout_vid[7]), (1, 5));
    assert_eq!(be32(layout_vid, 8), 0x7fff_efff);
    assert_eq!(be32(layout_vid, 12), 1);
    // 89 records in a 15 KiB LEB.
    let table = &image[1024..PEB];
    let record_at = |id: usize| &table[id * 172..][..172];
    assert_eq!(be32(record_at(0), 0), 4, "reserved_pebs");
    assert_eq!(be32(record_at(0), 4), 1, "alignment");
    assert_eq!(
        (record_at(0)[12], record_at(0)[13]),
        (1, 0),
        "vol_type, upd_marker"
    );
    assert_eq!(&record_at(0)[14..16], &[0, 8], "name_len");
    assert_eq!(&record_at(0)[16..25], b"rootfs_a\0");
    assert_eq!(record_at(2)[12], 2, "static");
    assert_eq!(record_at(3)[144], 1, "autoresize flag");
    for id in 0..89 {
        assert_eq!(be32(record_at(id), 168), ubi_crc(&record_at(id)[..168]));
    }
    assert_eq!(be32(record_at(4), 168), 0xf116_c36b, "an empty record");
    assert!(table[89 * 172..].iter().all(|byte| *byte == 0xff));
    let kernel_vid = &image[4 * PEB + 512..][..64];
    assert_eq!(kernel_vid[5], 2);
    assert_eq!(be32(kernel_vid, 20), 100);
    assert_eq!(be32(kernel_vid, 24), 1);
    assert_eq!(be32(kernel_vid, 32), ubi_crc(&[0x5a; 100]));
}

#[test]
fn a_gaia_sample_reads_back() {
    let bytes = gaia_sample(&rootfs(20_000), &[0x5a; 100]);
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(find_peb_size(&mut cursor).unwrap(), PEB as u32);
    let image = read_image(&mut cursor, PEB as u32).unwrap();
    assert_eq!(image.layout, layout16k());
    assert_eq!((image.pebs, image.image_seq), (5, SEQ));
    let volumes: Vec<(&str, u32, u32, Option<u32>)> = image
        .volumes
        .iter()
        .map(|v| (v.name.as_str(), v.reserved_lebs, v.image_lebs, v.first_peb))
        .collect();
    assert_eq!(
        volumes,
        [
            ("rootfs_a", 4, 2, Some(2)),
            ("rootfs_b", 4, 0, None),
            ("kernel", 1, 1, Some(4)),
            ("data", 1, 0, None),
        ]
    );
    assert_eq!(image.volume("kernel").unwrap().static_bytes, Some(100));
    assert!(image.volume("data").unwrap().autoresize);
    assert_eq!(image.reserved_pebs(), 2 + 2 + 10);
}

#[test]
fn a_gaia_sample_damage_is_named() {
    let read = |bytes: &[u8]| read_image(&mut Cursor::new(bytes), PEB as u32).unwrap_err();
    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    bytes[PEB + 4096] ^= 1;
    assert!(matches!(read(&bytes), ImageError::TablesDiffer));

    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    bytes[2 * PEB + 512 + 13] ^= 1;
    assert!(matches!(
        read(&bytes),
        ImageError::Header {
            peb: 2,
            error: HeaderError::Crc { .. }
        }
    ));

    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    bytes[3 * PEB + 1024] ^= 1; // the static volume's data
    assert!(matches!(
        read(&bytes),
        ImageError::StaticDataCrc { peb: 3, .. }
    ));

    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    bytes[2 * PEB..3 * PEB].fill(0xff);
    assert!(matches!(read(&bytes), ImageError::ErasedPeb { peb: 2 }));
}

#[test]
fn every_truncation_is_refused_or_read_without_panic() {
    let bytes = gaia_sample(&rootfs(20_000), &[0x5a; 100]);
    let mut lengths: Vec<usize> = (0..=bytes.len()).step_by(997).collect();
    for peb in 0..=5 {
        for delta in [0usize, 1, 511, 512, 1023] {
            if let Some(len) = (peb * PEB).checked_sub(delta) {
                lengths.push(len);
            }
            lengths.push(peb * PEB + delta);
        }
    }
    for len in lengths.into_iter().filter(|len| *len <= bytes.len()) {
        let result = read_image(&mut Cursor::new(&bytes[..len]), PEB as u32);
        if len % PEB != 0 {
            assert!(result.is_err(), "a {len}-byte image was read");
        }
        let _ = find_peb_size(&mut Cursor::new(&bytes[..len]));
    }
}

#[test]
fn every_header_bit_flip_is_refused() {
    let bytes = gaia_sample(&rootfs(20_000), &[0x5a; 100]);
    for peb in 0..5 {
        for at in (0..HEADER_SIZE).chain(512..512 + HEADER_SIZE) {
            let mut damaged = bytes.clone();
            damaged[peb * PEB + at] ^= 0x01;
            let result = read_image(&mut Cursor::new(&damaged), PEB as u32);
            assert!(
                result.is_err(),
                "PEB {peb} byte {at} flipped and still read"
            );
        }
    }
}

/// Patches a record of PEB `peb`'s volume table (record 0) and reseals it.
fn patch_record0(bytes: &mut [u8], peb: usize, patch: impl Fn(&mut [u8])) {
    let at = peb * PEB + 1024;
    let mut record = bytes[at..at + VTBL_RECORD_SIZE].to_vec();
    patch(&mut record);
    seal(&mut record);
    bytes[at..at + VTBL_RECORD_SIZE].copy_from_slice(&record);
}

#[test]
fn hostile_fields_are_refused_without_allocation() {
    let read = |bytes: &[u8]| read_image(&mut Cursor::new(bytes), PEB as u32).unwrap_err();

    // An EC header whose VID offset and data offset point past the PEB.
    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    let hostile = EcHeader {
        erase_counter: 0,
        vid_header_offset: 0xffff_fff8,
        data_offset: u32::MAX,
        image_seq: SEQ,
    };
    bytes[..HEADER_SIZE].copy_from_slice(&enc_ec(&hostile));
    assert!(matches!(read(&bytes), ImageError::Layout { peb: 0, .. }));

    // A static LEB claiming four gigabytes of data.
    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    let claim = VidHeader {
        data_size: u32::MAX,
        used_ebs: 1,
        ..vid(VolumeKind::Static, 2, 0)
    };
    bytes[3 * PEB + 512..3 * PEB + 512 + HEADER_SIZE].copy_from_slice(&enc_vid(&claim));
    assert!(matches!(
        read(&bytes),
        ImageError::StaticDataCrc { peb: 3, .. }
    ));

    // A volume id the table does not list (a valid id, so the header passes).
    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    let stray = vid(VolumeKind::Dynamic, 127, 0);
    bytes[2 * PEB + 512..2 * PEB + 512 + HEADER_SIZE].copy_from_slice(&enc_vid(&stray));
    assert!(matches!(
        read(&bytes),
        ImageError::UnlistedVolume { id: 127 }
    ));

    // A volume name length of 65535 in both table copies.
    let mut bytes = gaia_sample(&[1; 100], &[2; 10]);
    patch_record0(&mut bytes, 0, |record| {
        record[14..16].copy_from_slice(&0xffffu16.to_be_bytes())
    });
    patch_record0(&mut bytes, 1, |record| {
        record[14..16].copy_from_slice(&0xffffu16.to_be_bytes())
    });
    assert!(matches!(
        read(&bytes),
        ImageError::Record {
            id: 0,
            error: HeaderError::NameLength(65_535)
        }
    ));

    // A PEB size no flash has: refused before any buffer is made.
    let image = gaia_sample(&[1; 100], &[2; 10]);
    let error = read_image(&mut Cursor::new(&image), u32::MAX).unwrap_err();
    assert!(matches!(error, ImageError::PebSize { .. }), "{error}");
    let error = read_image(&mut Cursor::new(&image), 0).unwrap_err();
    assert!(
        matches!(error, ImageError::PebSize { peb_size: 0 }),
        "{error}"
    );

    // Noise is not an image.
    let noise: Vec<u8> = (0..PEB * 3).map(|i| (i * 7 + 3) as u8).collect();
    assert!(find_peb_size(&mut Cursor::new(&noise)).is_err());
    assert!(read_image(&mut Cursor::new(&noise), PEB as u32).is_err());
}

#[test]
fn headers_decode_what_they_encode_and_name_the_rest() {
    let ec = EcHeader {
        erase_counter: 7,
        vid_header_offset: 512,
        data_offset: 1024,
        image_seq: SEQ,
    };
    let bytes = enc_ec(&ec);
    assert_eq!(EcHeader::decode(&bytes), Ok(Some(ec)));
    assert_eq!(ec.validate(), Ok(()));
    let mut damaged = bytes;
    damaged[9] ^= 1;
    assert!(matches!(
        EcHeader::decode(&damaged),
        Err(HeaderError::Crc { .. })
    ));
    assert_eq!(EcHeader::decode(&[0xff; HEADER_SIZE]), Ok(None));
    assert!(matches!(
        EcHeader::decode(&[0; 10]),
        Err(HeaderError::TooShort { len: 10, .. })
    ));

    let static_vid = VidHeader {
        data_size: 100,
        used_ebs: 10,
        data_crc: 0xdead_beef,
        sqnum: 3,
        ..vid(VolumeKind::Static, 3, 9)
    };
    assert_eq!(
        VidHeader::decode(&enc_vid(&static_vid)),
        Ok(Some(static_vid))
    );
    assert_eq!(static_vid.validate(), Ok(()));
    assert!(matches!(
        VidHeader::decode(&bytes),
        Err(HeaderError::Magic {
            found: EC_MAGIC,
            ..
        })
    ));
    let mut typed = enc_vid(&static_vid);
    typed[5] = 9;
    seal(&mut typed);
    assert!(matches!(
        VidHeader::decode(&typed),
        Err(HeaderError::VolumeType { code: 9, .. })
    ));
    let dynamic_with_crc = VidHeader {
        data_crc: 1,
        ..vid(VolumeKind::Dynamic, 0, 0)
    };
    assert!(dynamic_with_crc.validate().is_err());

    let rec = record(40, VolumeKind::Dynamic, "rootfs_a", false);
    let encoded = enc_record(&rec);
    assert_eq!(VtblRecord::decode(&encoded), Ok(Some(rec)));
    assert_eq!(VtblRecord::decode(&enc_empty_record()), Ok(None));
    let mut damaged = encoded;
    damaged[20] ^= 1;
    assert!(matches!(
        VtblRecord::decode(&damaged),
        Err(HeaderError::Crc { .. })
    ));
    assert!(matches!(
        VtblRecord::decode(&encoded[..100]),
        Err(HeaderError::TooShort { .. })
    ));
}

#[test]
fn a_peb_that_cannot_hold_its_headers_is_refused() {
    let layout = PebLayout {
        peb_size: 4096,
        vid_header_offset: 4096,
        data_offset: 4096,
    };
    assert!(layout.validate().is_err());
    assert_eq!(layout.leb_size(), 0);
    assert_eq!(layout.vtbl_records(), 0);
}
