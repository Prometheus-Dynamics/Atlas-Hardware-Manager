//! CRC-32 (IEEE 802.3, reflected, polynomial 0xEDB88320), and UBI's use of
//! it: the kernel's `crc32_le(UBI_CRC32_INIT, ...)`, the standard CRC-32
//! without its final inversion.

const fn table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

const TABLE: [u32; 256] = table();

/// Continues a reflected CRC-32 from `crc` (no inversions).
pub fn update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc = TABLE[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    crc
}

/// The standard CRC-32 (zlib, PNG, Ethernet).
pub fn crc32(bytes: &[u8]) -> u32 {
    !update(!0, bytes)
}

/// UBI's header and data CRC.
pub fn ubi_crc(bytes: &[u8]) -> u32 {
    update(!0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        // Gaia's writer and the kernel agree on these (Gaia ubi.rs tests).
        assert_eq!(ubi_crc(b"123456789"), 0x340b_c6d9);
        assert_eq!(ubi_crc(&[0u8; 168]), 0xf116_c36b);
    }
}
