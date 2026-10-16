//! Whether the kernel can unpack an initramfs: an uncompressed cpio, or one
//! compressed with a format the Raze kernel decodes (zstd, gzip, xz). xz
//! must use a dictionary of at most 1 MiB (the kernel's initramfs decoder
//! didn't take level 9's 64 MiB: r54's trial never reached /init), and its
//! block header must carry both sizes (xz -T1 leaves them out, and r55's
//! didn't boot until the same cpio was packed with -T2, which writes them).
//! The slots writer refuses such a bundle before it is installed.

/// The largest xz dictionary the kernel's initramfs decoder is known to take.
pub const XZ_DICT_MAX: u64 = 1 << 20;

/// The initramfs's format (`cpio`, `zstd`, `gzip`, `xz (dict N KiB)`), or why
/// the kernel can't unpack it.
pub fn check(head: &[u8]) -> Result<String, String> {
    if head.starts_with(b"070701") || head.starts_with(b"070702") {
        return Ok("cpio".into());
    }
    if head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Ok("zstd".into());
    }
    if head.starts_with(&[0x1f, 0x8b]) {
        return Ok("gzip".into());
    }
    if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        let (dict, sizes) =
            xz_block(head).ok_or("an xz stream whose dictionary size can't be read")?;
        if !sizes {
            return Err(
                "xz without the sizes in its block header (xz -T1 writes it so): the kernel didn't unpack that as an initramfs; pack it with xz -T2 or later Gaia"
                    .into(),
            );
        }
        if dict > XZ_DICT_MAX {
            return Err(format!(
                "xz with a {} MiB dictionary: the kernel unpacks an initramfs only with up to {} MiB (xz -1, or --lzma2=dict=1MiB)",
                dict >> 20,
                XZ_DICT_MAX >> 20
            ));
        }
        return Ok(format!("xz (dict {} KiB)", dict >> 10));
    }
    Err("not a cpio archive, nor zstd, gzip or xz".into())
}

/// A multibyte integer as xz writes them (7 bits a byte, low first).
fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..63).step_by(7) {
        let b = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// The LZMA2 dictionary size from the first block header of an xz stream,
/// and whether that header carries both the compressed and uncompressed
/// sizes.
fn xz_block(head: &[u8]) -> Option<(u64, bool)> {
    // The 12-byte stream header, then the block header: its size byte,
    // flags (filter count, optional sizes), then each filter's id,
    // properties size and properties.
    let mut at = 12usize;
    let size = *head.get(at)?;
    if size == 0 {
        return None;
    }
    at += 1;
    let flags = *head.get(at)?;
    at += 1;
    let sizes = flags & 0xc0 == 0xc0;
    if flags & 0x40 != 0 {
        varint(head, &mut at)?;
    }
    if flags & 0x80 != 0 {
        varint(head, &mut at)?;
    }
    for _ in 0..(flags & 0x03) + 1 {
        let id = varint(head, &mut at)?;
        let props = varint(head, &mut at)? as usize;
        if id == 0x21 && props == 1 {
            let p = u32::from(*head.get(at)?);
            return match p {
                40 => Some((u64::from(u32::MAX), sizes)),
                0..=39 => Some((u64::from(2 | (p & 1)) << (p / 2 + 11), sizes)),
                _ => None,
            };
        }
        at += props;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start of an xz stream with one LZMA2 filter of props `p`, its
    /// block header with both sizes (as xz -T2 writes it).
    fn xz(p: u8) -> Vec<u8> {
        let mut v = vec![0xfd, b'7', b'z', b'X', b'Z', 0x00, 0x00, 0x01, 0, 0, 0, 0];
        // Size byte, flags (one filter, both sizes), the sizes, LZMA2.
        v.extend_from_slice(&[0x03, 0xc0, 0x80, 0x01, 0x90, 0x02, 0x21, 0x01, p, 0, 0, 0]);
        v
    }

    /// The same without the sizes (xz -T1).
    fn xz_unsized(p: u8) -> Vec<u8> {
        let mut v = vec![0xfd, b'7', b'z', b'X', b'Z', 0x00, 0x00, 0x01, 0, 0, 0, 0];
        v.extend_from_slice(&[0x02, 0x00, 0x21, 0x01, p, 0, 0, 0]);
        v
    }

    #[test]
    fn formats_the_kernel_unpacks() {
        assert_eq!(check(b"070701000000").unwrap(), "cpio");
        assert_eq!(check(&[0x28, 0xb5, 0x2f, 0xfd, 0]).unwrap(), "zstd");
        assert_eq!(check(&[0x1f, 0x8b, 8]).unwrap(), "gzip");
        // props 16: 1 MiB (xz -1); 8: 64 KiB.
        assert_eq!(check(&xz(16)).unwrap(), "xz (dict 1024 KiB)");
        assert_eq!(check(&xz(8)).unwrap(), "xz (dict 64 KiB)");
    }

    #[test]
    fn a_big_xz_dictionary_and_unknown_formats_are_refused() {
        // props 28: 64 MiB (xz -9).
        let refused = check(&xz(28)).unwrap_err();
        assert!(refused.contains("64 MiB"), "{refused}");
        assert!(check(b"hello").is_err());
        let unsized_ = check(&xz_unsized(16)).unwrap_err();
        assert!(unsized_.contains("without the sizes"), "{unsized_}");
    }
}
