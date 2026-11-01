//! Gaia's `slot-bundle.tar` (a plain ustar): where each member lies, so an
//! install reads what it needs straight from the file, without unpacking it
//! on a board with little room to spare.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Take};
use std::path::Path;

const BLOCK: u64 = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

fn octal(field: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(field).ok()?;
    let text = text.trim_matches(|c: char| c == '\0' || c == ' ');
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

/// The regular files of a tar, in order. A damaged header stops it with why.
pub fn members(tar: &mut (impl Read + Seek)) -> Result<Vec<Member>, String> {
    let mut found = Vec::new();
    let mut at = 0u64;
    let mut header = [0u8; BLOCK as usize];
    loop {
        tar.seek(SeekFrom::Start(at))
            .map_err(|error| format!("the bundle: {error}"))?;
        if tar.read_exact(&mut header).is_err() || header.iter().all(|byte| *byte == 0) {
            return Ok(found);
        }
        let stored = octal(&header[148..156]);
        let sum: u64 = header
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    u64::from(b' ')
                } else {
                    u64::from(*byte)
                }
            })
            .sum();
        if stored != Some(sum) {
            return Err(format!("the bundle's tar header at {at} is damaged"));
        }
        let field = |range: std::ops::Range<usize>| {
            let bytes = &header[range];
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        };
        let size = octal(&header[124..136])
            .ok_or_else(|| format!("the bundle's tar header at {at} has no size"))?;
        let prefix = field(345..500);
        let name = field(0..100);
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if matches!(header[156], 0 | b'0') {
            found.push(Member {
                name: name.trim_start_matches("./").to_string(),
                offset: at + BLOCK,
                size,
            });
        }
        at += BLOCK + size.div_ceil(BLOCK) * BLOCK;
    }
}

/// A member of a bundle on disk.
pub struct Bundle {
    path: std::path::PathBuf,
    members: Vec<Member>,
}

impl Bundle {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            members: members(&mut file)?,
        })
    }

    fn member(&self, name: &str) -> Result<&Member, String> {
        self.members
            .iter()
            .find(|member| member.name == name)
            .ok_or_else(|| format!("{} has no {name}", self.path.display()))
    }

    /// A member's bytes as a reader, and how many.
    pub fn reader(&self, name: &str) -> Result<(Take<File>, u64), String> {
        let member = self.member(name)?;
        let mut file =
            File::open(&self.path).map_err(|error| format!("{}: {error}", self.path.display()))?;
        file.seek(SeekFrom::Start(member.offset))
            .map_err(|error| format!("{}: {error}", self.path.display()))?;
        Ok((file.take(member.size), member.size))
    }

    /// A small member's bytes.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let (mut reader, size) = self.reader(name)?;
        let mut bytes = Vec::with_capacity(size as usize);
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| format!("{name}: {error}"))?;
        if bytes.len() as u64 != size {
            return Err(format!("{} is cut short at {name}", self.path.display()));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in files {
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..107].copy_from_slice(b"0000644");
            header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            header[156] = b'0';
            header[257..262].copy_from_slice(b"ustar");
            header[148..156].fill(b' ');
            let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
            header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.resize(out.len() + 1024, 0);
        out
    }

    #[test]
    fn members_are_found_and_read() {
        let path =
            std::env::temp_dir().join(format!("atlas-ubi-bundle-{}.tar", std::process::id()));
        let boot = vec![7u8; 1500];
        std::fs::write(&path, tar(&[("boot.img", &boot), ("./env-b.img", b"env")])).unwrap();
        let bundle = Bundle::open(&path).unwrap();
        assert_eq!(bundle.read("env-b.img").unwrap(), b"env");
        let (mut reader, size) = bundle.reader("boot.img").unwrap();
        let mut read = Vec::new();
        reader.read_to_end(&mut read).unwrap();
        assert_eq!((size, read), (1500, boot));
        assert!(bundle.read("nope").unwrap_err().contains("has no nope"));

        let mut damaged = tar(&[("boot.img", b"x")]);
        damaged[3] ^= 1;
        std::fs::write(&path, damaged).unwrap();
        assert!(Bundle::open(&path).err().unwrap().contains("damaged"));
        let _ = std::fs::remove_file(path);
    }
}
