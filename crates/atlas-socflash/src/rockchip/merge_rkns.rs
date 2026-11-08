//! The RKNS header of a `NEWIDB` loader (the UsbHead and FlashHead entries,
//! and the first block of an idblock).
//!
//! From rockchip-linux/u-boot 1c535d6 `tools/rkcommon.c`
//! `rkcommon_set_header0_v2` (357-428) and its `struct header0_info_v2`
//! (66-76) and `struct image_entry` (44-52): magic, `size_and_nimage`,
//! `boot_flag` = SHA-256, up to four images from the header's own size on,
//! each with size and sector offset, load address (0xFFFFFFFF unless the ini
//! gives one), counter and SHA-256, then SHA-256 of the header up to the
//! hash field.
//!
//! Inferred from Gaia's rock-4d rk3576_loader_v1.13.109.bin (the C source
//! has none of these): the header is `[SYSTEM] ALIGN` sectors long and the
//! first image follows it; `[BOOT1_PARAM] WORD_n` is the u32 at 88 + 4n
//! (WORD_2=0x100 sits at 96, in UsbHead and FlashHead alike); the image's
//! `LOAD_ADDR` is the `address` field and `FLAG` the `flag` field (FLAG=0
//! in the only file, so that placement is not distinguished by it).

use sha2::{Digest, Sha256};

use crate::rockchip::merge::MergeError;

pub(crate) const SECTOR: usize = 512;
const IMAGES_AT: usize = 120;
const IMAGE_SIZE: usize = 88;
const MAX_IMAGES: usize = 4;
const HASH_AT: usize = 1536;
const WORDS_AT: usize = 88;
/// `[BOOT1_PARAM]` holds WORD_0 to WORD_7 (the images start where WORD_8
/// would).
pub(crate) const BOOT1_WORDS: usize = (IMAGES_AT - WORDS_AT) / 4;

/// An image's `LOADn_PARAM` settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageParam {
    pub load_addr: u32,
    pub flag: u32,
}

impl Default for ImageParam {
    fn default() -> Self {
        Self {
            load_addr: 0xffff_ffff,
            flag: 0,
        }
    }
}

/// The RKNS header of `images` (each already padded): `head_size` bytes long,
/// images from the sector after it.
pub(crate) fn rkns_header(
    images: &[&[u8]],
    head_size: usize,
    words: &[u32; BOOT1_WORDS],
    params: &[ImageParam],
) -> Result<Vec<u8>, MergeError> {
    if images.len() > MAX_IMAGES {
        return Err(MergeError::Unsupported(format!(
            "{} images; an RKNS header holds {MAX_IMAGES}",
            images.len()
        )));
    }
    let mut header = vec![0u8; head_size];
    header[..4].copy_from_slice(b"RKNS");
    // [31:16] images, [15:0] the hash field's offset in words.
    let size_and_nimage = ((images.len() as u32) << 16) + (HASH_AT / 4) as u32;
    header[8..12].copy_from_slice(&size_and_nimage.to_le_bytes());
    header[12..16].copy_from_slice(&1u32.to_le_bytes()); // boot_flag: SHA-256
    for (index, word) in words.iter().enumerate() {
        let at = WORDS_AT + index * 4;
        header[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut sector = (head_size / SECTOR) as u32;
    for (index, image) in images.iter().enumerate() {
        let sectors = u32::try_from(image.len() / SECTOR)
            .ok()
            .filter(|count| *count <= 0xffff)
            .ok_or_else(|| MergeError::OutOfRange("an image is over 32 MiB".into()))?;
        let param = params.get(index).copied().unwrap_or_default();
        let at = IMAGES_AT + index * IMAGE_SIZE;
        header[at..at + 4].copy_from_slice(&((sectors << 16) + sector).to_le_bytes());
        header[at + 4..at + 8].copy_from_slice(&param.load_addr.to_le_bytes());
        header[at + 8..at + 12].copy_from_slice(&param.flag.to_le_bytes());
        header[at + 12..at + 16].copy_from_slice(&(index as u32 + 1).to_le_bytes());
        header[at + 24..at + 56].copy_from_slice(&Sha256::digest(image));
        sector += sectors;
    }
    let hash = Sha256::digest(&header[..HASH_AT]);
    header[HASH_AT..HASH_AT + 32].copy_from_slice(&hash);
    Ok(header)
}
