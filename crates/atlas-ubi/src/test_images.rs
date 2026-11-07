//! Test images laid out the way Gaia's writer lays them out, built with
//! gaia-formats' encoder, and read back through the crate's reader.

use gaia_formats::ubi::{
    EcHeader, PebLayout, VidHeader, VolumeKind, VtblRecord, encode_peb, encode_volume_table,
    find_peb_size, programmed_len, read_image,
};
use gaia_formats::ubi::{Image, LAYOUT_VOLUME_ID};

pub const SEQ: u32 = 0x1234_5678;

/// A small image as Gaia writes it (its ubi.rs, field for field): 16 KiB
/// PEBs, 512-byte pages, rootfs_a with data, rootfs_b and data empty.
pub fn sample(rootfs: &[u8]) -> (PebLayout, Vec<u8>) {
    let layout = PebLayout {
        peb_size: 16 * 1024,
        vid_header_offset: 512,
        data_offset: 1024,
    };
    let leb = layout.leb_size() as usize;
    let volumes: Vec<(u32, VtblRecord)> = [
        ("rootfs_a", 4, false),
        ("rootfs_b", 4, false),
        ("data", 1, true),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, (name, reserved, autoresize))| {
        let kind = VolumeKind::Dynamic;
        (id as u32, VtblRecord::new(reserved, kind, name, autoresize))
    })
    .collect();
    let table = encode_volume_table(&layout, &volumes).unwrap();
    let ec = EcHeader {
        erase_counter: 0,
        vid_header_offset: layout.vid_header_offset,
        data_offset: layout.data_offset,
        image_seq: SEQ,
    };
    let peb = |volume_id: u32, lnum: u32, compat: u8, data: &[u8]| {
        let vid = VidHeader {
            compat,
            ..VidHeader::new(VolumeKind::Dynamic, volume_id, lnum)
        };
        encode_peb(&layout, &ec, &vid, data)
    };
    let mut image = Vec::new();
    image.extend(peb(LAYOUT_VOLUME_ID, 0, 5, &table));
    image.extend(peb(LAYOUT_VOLUME_ID, 1, 5, &table));
    for (lnum, chunk) in rootfs.chunks(leb).enumerate() {
        image.extend(peb(0, lnum as u32, 0, chunk));
    }
    (layout, image)
}

#[test]
fn a_gaia_style_image_reads_back() {
    let rootfs: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    let (layout, bytes) = sample(&rootfs);
    let mut cursor = std::io::Cursor::new(&bytes);
    assert_eq!(find_peb_size(&mut cursor).unwrap(), 16 * 1024);
    let image: Image = read_image(&mut cursor, 16 * 1024).unwrap();
    assert_eq!(image.layout, layout);
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
    assert!(error.to_string().contains("differ"), "{error}");
    let (_, mut bytes) = sample(&[1; 100]);
    bytes[2 * 16 * 1024 + 512 + 13] ^= 1; // a VID header's lnum
    let error = read_image(&mut std::io::Cursor::new(&bytes), 16 * 1024).unwrap_err();
    assert!(error.to_string().contains("CRC"), "{error}");
    let error = read_image(&mut std::io::Cursor::new(&bytes[..1000]), 16 * 1024).unwrap_err();
    assert!(error.to_string().contains("whole number"), "{error}");
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
