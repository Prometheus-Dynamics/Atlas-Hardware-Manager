//! Gaia's slot bundle (`slot-bundle.tar`, a plain ustar): where each member
//! lies in the file, so a reader takes what it needs straight from the
//! bundle without unpacking it.
//!
//! The tar is untrusted: every member's extent is checked against the file's
//! length before it is recorded, and a header is accepted only with a valid
//! checksum. A member that the caller reads whole is capped at
//! [`SMALL_MEMBER_MAX`]; larger ones are read through [`Bundle::reader`].

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Take};
use std::path::{Path, PathBuf};

const BLOCK: u64 = 512;

/// The most [`Bundle::read`] returns in one piece.
pub const SMALL_MEMBER_MAX: u64 = 16 * 1024 * 1024;

/// A regular file in the bundle: where its bytes lie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// Why a bundle cannot be read.
#[derive(Debug)]
pub enum BundleError {
    Io(std::io::Error),
    /// A header whose checksum does not match.
    Damaged {
        at: u64,
    },
    /// A header whose size field is not octal.
    NoSize {
        at: u64,
    },
    /// A member or header that runs past the end of the file.
    Truncated {
        at: u64,
    },
    Missing {
        name: String,
    },
    TooLarge {
        name: String,
        size: u64,
    },
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "the bundle: {error}"),
            Self::Damaged { at } => write!(f, "the bundle's tar header at {at} is damaged"),
            Self::NoSize { at } => write!(f, "the bundle's tar header at {at} has no size"),
            Self::Truncated { at } => write!(f, "the bundle is cut short at {at}"),
            Self::Missing { name } => write!(f, "the bundle has no {name}"),
            Self::TooLarge { name, size } => write!(
                f,
                "{name} is {size} bytes; read it through the reader, the limit for one read is {SMALL_MEMBER_MAX}"
            ),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<std::io::Error> for BundleError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// A field of octal digits, space or NUL padded. `None` when it is not octal.
fn octal(field: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(field).ok()?;
    let text = text.trim_matches(|c: char| c == '\0' || c == ' ');
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

/// The regular files of a tar, in order. A damaged or cut-short header stops
/// the walk with why.
pub fn members(tar: &mut (impl Read + Seek)) -> Result<Vec<Member>, BundleError> {
    let len = tar.seek(SeekFrom::End(0))?;
    let mut found = Vec::new();
    let mut at = 0u64;
    let mut header = [0u8; BLOCK as usize];
    loop {
        // A tar ends with zero blocks, or at the end of the file.
        if at >= len {
            return Ok(found);
        }
        if at + BLOCK > len {
            return Err(BundleError::Truncated { at });
        }
        tar.seek(SeekFrom::Start(at))?;
        tar.read_exact(&mut header)?;
        if header.iter().all(|byte| *byte == 0) {
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
            return Err(BundleError::Damaged { at });
        }
        let field = |range: std::ops::Range<usize>| {
            let bytes = &header[range];
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        };
        let size = octal(&header[124..136]).ok_or(BundleError::NoSize { at })?;
        let start = at + BLOCK;
        // The member's bytes must lie inside the file. `size` is then at most
        // the file's length, so the rounding below cannot overflow.
        if size > len - start {
            return Err(BundleError::Truncated { at });
        }
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
                offset: start,
                size,
            });
        }
        at = start + size.div_ceil(BLOCK) * BLOCK;
    }
}

/// A slot bundle on disk, with its members located.
#[derive(Debug)]
pub struct Bundle {
    path: PathBuf,
    members: Vec<Member>,
}

impl Bundle {
    pub fn open(path: &Path) -> Result<Self, BundleError> {
        let mut file = File::open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            members: members(&mut file)?,
        })
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    fn member(&self, name: &str) -> Result<&Member, BundleError> {
        self.members
            .iter()
            .find(|member| member.name == name)
            .ok_or_else(|| BundleError::Missing {
                name: name.to_string(),
            })
    }

    /// A member's bytes as a reader, and how many there are.
    pub fn reader(&self, name: &str) -> Result<(Take<File>, u64), BundleError> {
        let member = self.member(name)?;
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(member.offset))?;
        Ok((file.take(member.size), member.size))
    }

    /// A small member's bytes, capped at [`SMALL_MEMBER_MAX`].
    pub fn read(&self, name: &str) -> Result<Vec<u8>, BundleError> {
        let member = self.member(name)?;
        if member.size > SMALL_MEMBER_MAX {
            return Err(BundleError::TooLarge {
                name: name.to_string(),
                size: member.size,
            });
        }
        let (mut reader, size) = self.reader(name)?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        if bytes.len() as u64 != size {
            return Err(BundleError::Truncated { at: member.offset });
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A ustar with the given regular files, each padded to whole blocks.
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

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("atlas-ubi-{tag}-{}.tar", std::process::id()))
    }

    #[test]
    fn members_are_located_and_read() {
        let boot = vec![7u8; 1500];
        let bytes = tar(&[("boot.img", &boot), ("./env-b.img", b"env")]);
        let found = members(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(
            found,
            [
                Member {
                    name: "boot.img".into(),
                    offset: 512,
                    size: 1500
                },
                Member {
                    name: "env-b.img".into(),
                    offset: 512 + 1536 + 512,
                    size: 3
                },
            ]
        );

        let path = temp_path("members");
        std::fs::write(&path, &bytes).unwrap();
        let bundle = Bundle::open(&path).unwrap();
        assert_eq!(bundle.read("env-b.img").unwrap(), b"env");
        let (mut reader, size) = bundle.reader("boot.img").unwrap();
        let mut read = Vec::new();
        reader.read_to_end(&mut read).unwrap();
        assert_eq!((size, read), (1500, boot));
        assert!(matches!(
            bundle.read("nope"),
            Err(BundleError::Missing { .. })
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_damaged_header_is_named() {
        let mut bytes = tar(&[("boot.img", b"x")]);
        bytes[3] ^= 1;
        assert!(matches!(
            members(&mut Cursor::new(&bytes)),
            Err(BundleError::Damaged { at: 0 })
        ));
    }

    #[test]
    fn a_cut_short_bundle_is_refused() {
        let bytes = tar(&[("boot.img", &[1u8; 1500])]);
        // Cut inside the member's data.
        let cut = &bytes[..512 + 700];
        assert!(matches!(
            members(&mut Cursor::new(cut)),
            Err(BundleError::Truncated { at: 0 })
        ));
        // Cut inside a header.
        assert!(matches!(
            members(&mut Cursor::new(&bytes[..300])),
            Err(BundleError::Truncated { at: 0 })
        ));
        // Cut exactly after the member's data (no end blocks) is still a whole
        // bundle: a tar may end there.
        assert_eq!(
            members(&mut Cursor::new(&bytes[..512 + 1536]))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_hostile_size_is_refused_without_allocation() {
        let mut bytes = tar(&[("boot.img", b"x")]);
        // Size field: 0o77777777777, about 8 GiB, with a valid checksum.
        bytes[124..135].copy_from_slice(b"77777777777");
        bytes[148..156].fill(b' ');
        let sum: u32 = bytes[..512].iter().map(|byte| u32::from(*byte)).sum();
        bytes[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        assert!(matches!(
            members(&mut Cursor::new(&bytes)),
            Err(BundleError::Truncated { at: 0 })
        ));
        // A size that is not octal.
        bytes[124..135].copy_from_slice(b"zzzzzzzzzzz");
        bytes[148..156].fill(b' ');
        let sum: u32 = bytes[..512].iter().map(|byte| u32::from(*byte)).sum();
        bytes[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        assert!(matches!(
            members(&mut Cursor::new(&bytes)),
            Err(BundleError::NoSize { at: 0 })
        ));
    }

    #[test]
    fn a_member_too_large_for_one_read_is_refused() {
        let path = temp_path("large");
        let data = vec![0u8; SMALL_MEMBER_MAX as usize + 1];
        std::fs::write(&path, tar(&[("big.img", &data)])).unwrap();
        let bundle = Bundle::open(&path).unwrap();
        assert!(matches!(
            bundle.read("big.img"),
            Err(BundleError::TooLarge { .. })
        ));
        // Through the reader it is still there.
        let (_, size) = bundle.reader("big.img").unwrap();
        assert_eq!(size, SMALL_MEMBER_MAX + 1);
        let _ = std::fs::remove_file(path);
    }
}
