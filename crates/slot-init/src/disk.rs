//! A slots build's whole-disk image (`<id>.img`, decompressed on the way
//! in) split into its partitions, one sparse file each, for `slots
//! stage-image`: the zero runs (most of the 1 GiB boot FAT, the data
//! partition's free space) stay holes, so /data holds only what the disk
//! carries, and each file mounts as it is (`mount -o ro,loop`).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Holes are made per block of this size that is all zeros.
const BLOCK: usize = 64 * 1024;
const SECTOR: u64 = 512;

/// One MBR partition: where it starts and how long it is, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    pub start: u64,
    pub size: u64,
}

/// The MBR's primary partitions, in table order; empty entries left out.
pub fn mbr_partitions(sector: &[u8]) -> Result<Vec<Partition>, String> {
    if sector.len() < 512 || sector[510..512] != [0x55, 0xaa] {
        return Err("not a disk image: no MBR".into());
    }
    let mut parts = Vec::new();
    for entry in sector[446..510].as_chunks::<16>().0 {
        let lba = u32::from_le_bytes(entry[8..12].try_into().expect("4 bytes"));
        let count = u32::from_le_bytes(entry[12..16].try_into().expect("4 bytes"));
        if entry[4] != 0 && count != 0 {
            parts.push(Partition {
                start: u64::from(lba) * SECTOR,
                size: u64::from(count) * SECTOR,
            });
        }
    }
    Ok(parts)
}

/// Splits the disk image on `input` into one sparse file per MBR partition,
/// `<dir>/p1.img`, `<dir>/p2.img` and so on, each as long as its partition
/// (so a filesystem whose tail the build truncated still mounts), with the
/// all-zero blocks left as holes. Returns the files' paths in table order.
pub fn unpack(mut input: impl Read, dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut first = vec![0u8; 512];
    let read = fill(&mut input, &mut first).map_err(|e| format!("the image: {e}"))?;
    let parts = mbr_partitions(&first[..read])?;
    if parts.is_empty() {
        return Err("the disk image has no partitions".into());
    }
    let mut files = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let path = dir.join(format!("p{}.img", i + 1));
        let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.set_len(part.size)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        files.push((path, file, *part));
    }
    let mut buffer = vec![0u8; BLOCK];
    let mut at = SECTOR;
    loop {
        let filled = fill(&mut input, &mut buffer).map_err(|e| format!("the image: {e}"))?;
        if filled == 0 {
            break;
        }
        let end = at + filled as u64;
        for (path, file, part) in &mut files {
            // The overlap of [at, end) with this partition.
            let from = at.max(part.start);
            let to = end.min(part.start + part.size);
            if from >= to {
                continue;
            }
            let bytes = &buffer[(from - at) as usize..(to - at) as usize];
            if bytes.iter().all(|&byte| byte == 0) {
                continue;
            }
            let fail = |e: io::Error| format!("{}: {e}", path.display());
            file.seek(SeekFrom::Start(from - part.start))
                .map_err(fail)?;
            file.write_all(bytes).map_err(fail)?;
        }
        at = end;
    }
    // Gaia truncates the disk after the last data: a partition may run past
    // the end, but none may start there.
    if let Some((path, ..)) = files.iter().find(|(_, _, part)| part.start >= at) {
        return Err(format!(
            "the disk image is cut short: {} starts past its end",
            path.display()
        ));
    }
    for (path, file, _) in &files {
        file.sync_all()
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(files.into_iter().map(|(path, ..)| path).collect())
}

/// Reads until `buffer` is full or the input ends.
fn fill(input: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match input.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mbr(entries: &[(u8, u32, u32)]) -> Vec<u8> {
        let mut sector = vec![0u8; 512];
        for (i, (kind, lba, count)) in entries.iter().enumerate() {
            let at = 446 + i * 16;
            sector[at + 4] = *kind;
            sector[at + 8..at + 12].copy_from_slice(&lba.to_le_bytes());
            sector[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        }
        sector[510] = 0x55;
        sector[511] = 0xaa;
        sector
    }

    #[test]
    fn the_partitions_come_from_the_mbr() {
        let sector = mbr(&[(0x0c, 2048, 2_097_152), (0x83, 2_099_200, 4096)]);
        assert_eq!(
            mbr_partitions(&sector).unwrap(),
            [
                Partition {
                    start: 1_048_576,
                    size: 1_073_741_824
                },
                Partition {
                    start: 1_074_790_400,
                    size: 2_097_152
                },
            ]
        );
        assert!(mbr_partitions(&[0u8; 512]).is_err());
    }

    #[test]
    fn each_partition_becomes_a_sparse_file_of_its_size() {
        // p1: sectors 2048..4096 (1 MiB at 1 MiB); p2: 4096..8192, past the
        // 3 MiB the disk was cut to.
        let mut disk = mbr(&[(0x0c, 2048, 2048), (0x83, 4096, 4096)]);
        disk.resize(3 * 1024 * 1024, 0);
        disk[1_048_576..1_048_580].copy_from_slice(b"FAT!");
        disk[2_097_152 + 7] = 0xee;
        disk[3 * 1024 * 1024 - 1] = 1;
        let dir = std::env::temp_dir().join(format!("slot-init-unpack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = unpack(disk.as_slice(), &dir).unwrap();
        assert_eq!(files, [dir.join("p1.img"), dir.join("p2.img")]);
        let p1 = std::fs::read(&files[0]).unwrap();
        assert_eq!(p1, disk[1_048_576..2_097_152]);
        let p2 = std::fs::read(&files[1]).unwrap();
        assert_eq!(p2.len(), 2 * 1024 * 1024);
        assert_eq!(p2[..1_048_576], disk[2_097_152..]);
        assert!(p2[1_048_576..].iter().all(|&b| b == 0));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // Holes: a few 64 KiB blocks hold data.
            let used = |path: &Path| std::fs::metadata(path).unwrap().blocks() * 512;
            assert!(used(&files[0]) + used(&files[1]) < 512 * 1024);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_disk_cut_before_a_partition_is_refused() {
        let mut disk = mbr(&[(0x0c, 2048, 2048), (0x83, 8192, 2048)]);
        disk.resize(2 * 1024 * 1024, 1);
        let dir = std::env::temp_dir().join(format!("slot-init-cut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = unpack(disk.as_slice(), &dir).unwrap_err();
        std::fs::remove_dir_all(dir).unwrap();
        assert!(error.contains("cut short"), "{error}");
    }
}
