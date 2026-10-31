//! The files a flash needs: the vendor loader the board record names, and
//! where an image's UBI region starts.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use atlas_ubi::headers::{EcHeader, HEADER_SIZE, Read as Header};

/// `*` matches any run of characters; nothing else is special.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || !name[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// The loader matching `pattern` in the first of `dirs` that has one (the
/// newest by name there, so `v1.16` wins over `v1.15`). Atlas never
/// downloads one.
pub fn find_loader(pattern: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| glob_match(pattern, name))
            })
            .collect();
        found.sort();
        found.pop()
    })
}

/// Where a SPI-NAND image's UBI region starts: the first eraseblock
/// boundary holding a valid erase-counter header, with the next one valid
/// too. `None` when the image has none (it's written as is).
pub fn find_ubi_start(path: &Path, peb_size: u32) -> std::io::Result<Option<u64>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let peb = u64::from(peb_size);
    let valid = |file: &mut File, at: u64| -> std::io::Result<bool> {
        if at + HEADER_SIZE as u64 > length {
            return Ok(false);
        }
        let mut header = [0u8; HEADER_SIZE];
        file.seek(SeekFrom::Start(at))?;
        file.read_exact(&mut header)?;
        Ok(matches!(EcHeader::read(&header), Header::Header(_)))
    };
    let mut at = 0;
    while at + 2 * peb <= length {
        if valid(&mut file, at)? && valid(&mut file, at + peb)? {
            return Ok(Some(at));
        }
        at += peb;
    }
    Ok(None)
}

/// A window of a file, `start..start + length`, as its own Read + Seek.
pub struct Section {
    file: File,
    start: u64,
    length: u64,
    position: u64,
}

impl Section {
    pub fn open(path: &Path, start: u64, length: u64) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(start))?;
        Ok(Self {
            file,
            start,
            length,
            position: 0,
        })
    }
}

impl Read for Section {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let left = self.length - self.position;
        let take = (buffer.len() as u64).min(left) as usize;
        let read = self.file.read(&mut buffer[..take])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for Section {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(at) => at as i128,
            SeekFrom::End(delta) => self.length as i128 + delta as i128,
            SeekFrom::Current(delta) => self.position as i128 + delta as i128,
        };
        if target < 0 || target > self.length as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek outside the section",
            ));
        }
        self.position = target as u64;
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas-soc-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn globs() {
        assert!(glob_match(
            "rk3588_spl_loader_v*.bin",
            "rk3588_spl_loader_v1.15.113.bin"
        ));
        assert!(!glob_match(
            "rk3588_spl_loader_v*.bin",
            "rk3576_spl_loader_v1.0.bin"
        ));
        assert!(glob_match("download*.bin", "download.bin"));
        assert!(glob_match("a*b*c", "a-x-b-y-c"));
        assert!(!glob_match("a*b*c", "a-x-c"));
        assert!(glob_match("exact.bin", "exact.bin"));
    }

    #[test]
    fn the_newest_loader_in_the_first_dir_that_has_one() {
        let first = temp_dir("loaders-a");
        let second = temp_dir("loaders-b");
        std::fs::write(second.join("rk3588_spl_loader_v1.16.bin"), b"x").unwrap();
        assert_eq!(
            find_loader("rk3588_spl_loader_v*.bin", &[first.clone(), second.clone()]),
            Some(second.join("rk3588_spl_loader_v1.16.bin"))
        );
        std::fs::write(first.join("rk3588_spl_loader_v1.14.bin"), b"x").unwrap();
        std::fs::write(first.join("rk3588_spl_loader_v1.15.bin"), b"x").unwrap();
        assert_eq!(
            find_loader("rk3588_spl_loader_v*.bin", &[first.clone(), second.clone()]),
            Some(first.join("rk3588_spl_loader_v1.15.bin"))
        );
        assert_eq!(
            find_loader("nothing*.bin", std::slice::from_ref(&first)),
            None
        );
        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    #[test]
    fn the_ubi_region_after_the_loaders() {
        let dir = temp_dir("ubi-start");
        let peb = 16 * 1024u32;
        let header = EcHeader {
            erase_counter: 0,
            vid_header_offset: 512,
            data_offset: 1024,
            image_seq: 1,
        }
        .encode();
        // Two eraseblocks of loaders, then three UBI PEBs.
        let mut image = vec![0x11u8; 2 * peb as usize];
        for _ in 0..3 {
            let mut block = vec![0xffu8; peb as usize];
            block[..64].copy_from_slice(&header);
            image.extend(block);
        }
        let path = dir.join("nand.img");
        std::fs::write(&path, &image).unwrap();
        assert_eq!(
            find_ubi_start(&path, peb).unwrap(),
            Some(2 * u64::from(peb))
        );
        let mut section = Section::open(&path, 2 * u64::from(peb), 3 * u64::from(peb)).unwrap();
        assert_eq!(section.seek(SeekFrom::End(0)).unwrap(), 3 * u64::from(peb));
        section.seek(SeekFrom::Start(0)).unwrap();
        let mut first = [0u8; 64];
        section.read_exact(&mut first).unwrap();
        assert_eq!(first, header);
        std::fs::write(&path, vec![0u8; 4 * peb as usize]).unwrap();
        assert_eq!(find_ubi_start(&path, peb).unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
