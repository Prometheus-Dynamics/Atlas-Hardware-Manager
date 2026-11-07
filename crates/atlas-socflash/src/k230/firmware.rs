//! Canaan's K230 firmware head, which the boot ROM checks on what it boots
//! from storage (the SPL at 1 MiB and its copy at 1.5 MiB on SD and eMMC):
//! "K230", a little-endian length and encryption type, the SHA-256 of the
//! body, then the body from 0x210 (a 4-byte version, then the code).
//! Checked on Gaia's K230D Zero sdcard.img (2026-10-31): both heads valid.
//! UNCONFIRMED by k230_flash (33a75f0): that tool never looks at a firmware
//! head (no "K230" magic, 0x210 offset or SHA-256 appears in it), so this
//! layout rests on the SDK notes and that one image; the boot ROM's own
//! check is not public.

use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 4] = b"K230";
pub const BODY_OFFSET: usize = 0x210;
/// Where Canaan's SD and eMMC layout puts the SPL, and its copy.
pub const SPL_OFFSETS: [u64; 2] = [0x10_0000, 0x18_0000];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    /// A head whose SHA-256 matches its body.
    Valid {
        length: u32,
        encryption: u32,
    },
    /// No head here.
    Missing,
    Damaged(String),
}

/// Checks the head at the start of `bytes` (which must hold its body too).
pub fn check(bytes: &[u8]) -> Head {
    if bytes.len() < BODY_OFFSET || &bytes[..4] != MAGIC {
        return Head::Missing;
    }
    let length = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let encryption = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let Some(body) = bytes.get(BODY_OFFSET..BODY_OFFSET + length as usize) else {
        return Head::Damaged(format!("its body of {length} bytes runs past the image"));
    };
    if Sha256::digest(body).as_slice() != &bytes[12..44] {
        return Head::Damaged("its SHA-256 doesn't match its body".into());
    }
    Head::Valid { length, encryption }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_research_vector_and_damage() {
        // A 16-byte zero payload (body: version 0, then the payload); the
        // research spec printed this digest without its last digit.
        let body = [0u8; 20];
        let mut head = vec![0u8; BODY_OFFSET];
        head[..4].copy_from_slice(MAGIC);
        head[4..8].copy_from_slice(&20u32.to_le_bytes());
        head[12..44].copy_from_slice(&Sha256::digest(body));
        assert_eq!(
            crate::hex(&head[12..44]),
            "de47c9b27eb8d300dbb5f2c353e632c393262cf06340c4fa7f1b40c4cbd36f90"
        );
        head.extend_from_slice(&body);
        assert_eq!(
            check(&head),
            Head::Valid {
                length: 20,
                encryption: 0
            }
        );
        head[BODY_OFFSET + 5] = 1;
        assert!(matches!(check(&head), Head::Damaged(_)));
        assert_eq!(check(&[0u8; 1024]), Head::Missing);
    }
}
