//! Whether the kernel can unpack an initramfs on a Raze: an uncompressed
//! cpio, or one compressed with zstd or gzip. Not xz: three xz encodings
//! stopped the kernel before /init (r54: a 64 MiB dictionary; r55: no sizes
//! in the block header; r56: a 16-byte block header), while zstd -19 booted
//! from the first try. The slots writer refuses such a bundle before it is
//! installed.

/// The initramfs's format (`cpio`, `zstd`, `gzip`), or why the kernel can't
/// unpack it.
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
        return Err(
            "an xz initramfs isn't supported on the Raze: three xz encodings failed to boot (r54, r55, r56); pack it with zstd"
                .into(),
        );
    }
    Err("not a cpio archive, nor zstd or gzip".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_kernel_unpacks() {
        assert_eq!(check(b"070701000000").unwrap(), "cpio");
        assert_eq!(check(&[0x28, 0xb5, 0x2f, 0xfd, 0]).unwrap(), "zstd");
        assert_eq!(check(&[0x1f, 0x8b, 8]).unwrap(), "gzip");
    }

    #[test]
    fn xz_and_unknown_formats_are_refused() {
        let xz = check(&[0xfd, b'7', b'z', b'X', b'Z', 0x00, 0, 1]).unwrap_err();
        assert!(xz.contains("isn't supported"), "{xz}");
        assert!(check(b"hello").is_err());
    }
}
