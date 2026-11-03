//! Against tests/data/sparse-fixture.img.xz (see make-sparse-fixture.sh):
//! MBR, FAT16 at 1 MiB (6 MiB, 1 KiB clusters, 104 of 6103 in use) and ext4
//! at 8 MiB (8 MiB, 4 KiB blocks, 844 of 2048 free).

use super::*;

const FAT_AT: u64 = 1 << 20;
const FAT_LEN: u64 = 6 << 20;
const EXT4_AT: u64 = 8 << 20;
const EXT4_LEN: u64 = 8 << 20;

fn fixture() -> Vec<u8> {
    let xz: &[u8] = include_bytes!("../../tests/data/sparse-fixture.img.xz");
    let mut image = Vec::new();
    lzma_rs::xz_decompress(&mut { xz }, &mut image).unwrap();
    image
}

fn map_of(image: &[u8], from: usize) -> FreeMap {
    let mut map = FreeMap::new();
    for (index, chunk) in image[from..].chunks(64 << 10).enumerate() {
        map.observe((from + index * (64 << 10)) as u64, chunk);
    }
    map
}

/// Free units of `unit` bytes in `range`.
fn free_units(map: &FreeMap, range: Range<u64>, unit: u64) -> u64 {
    range
        .step_by(unit as usize)
        .filter(|at| map.free(*at..at + unit))
        .count() as u64
}

#[test]
fn crc32c_is_linuxs() {
    // The standard CRC-32C check value, with the inversions ext4 leaves out.
    assert_eq!(!crc32c(!0, b"123456789"), 0xE306_9283);
}

#[test]
fn free_space_is_what_the_filesystems_say() {
    let image = fixture();
    let map = map_of(&image, 0);
    assert_eq!(free_units(&map, EXT4_AT..EXT4_AT + EXT4_LEN, 4096), 844);
    assert_eq!(free_units(&map, FAT_AT..FAT_AT + FAT_LEN, 1024), 6103 - 104);
    // The table, the gap before the first partition, FAT's own tables and
    // root directory, and ext4's metadata are never free.
    assert!(!map.free(0..4096));
    assert!(!map.free(4096..8192));
    assert!(!map.free(FAT_AT..FAT_AT + 4096));
    assert!(!map.free(EXT4_AT..EXT4_AT + 4096));
}

#[test]
fn a_files_own_zeros_are_written() {
    let image = fixture();
    let map = map_of(&image, 0);
    // ZEROS.BIN in the FAT's root directory: its first cluster and size.
    let boot = &image[FAT_AT as usize..];
    let (sector, reserved, copies, per_fat) = (
        le16(boot, 11),
        le16(boot, 14),
        u64::from(boot[16]),
        le16(boot, 22),
    );
    let root = FAT_AT + (reserved + copies * per_fat) * sector;
    let data = root + (le16(boot, 17) * 32).div_ceil(sector) * sector;
    let entry = image[root as usize..]
        .chunks(32)
        .take(16)
        .find(|entry| &entry[..11] == b"ZEROS   BIN")
        .unwrap();
    let (cluster, size) = (le16(entry, 26), le32(entry, 28));
    assert_eq!(size, 65536);
    let start = data + (cluster - 2) * 1024;
    assert!(
        image[start as usize..(start + size) as usize]
            .iter()
            .all(|byte| *byte == 0)
    );
    assert_eq!(free_units(&map, start..start + size, 1024), 0);
    let (runs, _) = runs(&map, 0, &image);
    assert!(
        runs.iter()
            .any(|run| run.start as u64 <= start && run.end as u64 >= start + size)
    );
}

#[test]
fn only_zeros_are_skipped_and_most_of_the_image_is() {
    let image = fixture();
    let map = map_of(&image, 0);
    let (runs, skipped) = runs(&map, 0, &image);
    let written: usize = runs.iter().map(|run| run.len()).sum();
    assert_eq!(written as u64 + skipped, image.len() as u64);
    assert!(skipped > 9 << 20, "skipped {skipped}");
    let mut kept = vec![false; image.len()];
    for run in &runs {
        kept[run.clone()].fill(true);
    }
    assert!(
        image
            .iter()
            .zip(&kept)
            .all(|(byte, kept)| *kept || *byte == 0)
    );
}

#[test]
fn a_bitmap_or_fat_that_doesnt_check_out_keeps_its_partition_whole() {
    let mut image = fixture();
    // ext4's block bitmap is at block 2 of its partition.
    image[EXT4_AT as usize + 2 * 4096 + 100] ^= 0x10;
    // The second FAT copy differs from the first.
    let boot = FAT_AT as usize;
    let second = boot + (le16(&image[boot..], 14) + le16(&image[boot..], 22)) as usize * 512;
    image[second + 40] ^= 0x01;
    let map = map_of(&image, 0);
    assert_eq!(free_units(&map, EXT4_AT..EXT4_AT + EXT4_LEN, 4096), 0);
    assert_eq!(free_units(&map, FAT_AT..FAT_AT + FAT_LEN, 1024), 0);
}

#[test]
fn without_the_table_nothing_is_free() {
    let image = fixture();
    // The stream starts past the table: nothing is known.
    let map = map_of(&image, 4096);
    assert_eq!(free_units(&map, 0..image.len() as u64, 4096), 0);
    // No signature: nothing either.
    let mut image = image;
    image[510] = 0;
    let map = map_of(&image, 0);
    assert_eq!(free_units(&map, 0..image.len() as u64, 4096), 0);
}
