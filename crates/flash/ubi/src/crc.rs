//! UBI's CRC: the kernel's `crc32_le(UBI_CRC32_INIT, ...)`, the reflected
//! CRC-32 (polynomial `0xedb88320`) with no initial or final inversion
//! beyond the register starting at all ones.

const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                0xedb8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
};

/// Continues a UBI CRC register from `crc`. Feeding a stream in pieces gives
/// the same value as feeding it whole, starting from `!0`.
pub fn update(crc: u32, bytes: &[u8]) -> u32 {
    bytes.iter().fold(crc, |crc, byte| {
        TABLE[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8)
    })
}

/// The UBI CRC of `bytes`.
pub fn ubi_crc(bytes: &[u8]) -> u32 {
    update(!0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values() {
        assert_eq!(ubi_crc(b"123456789"), 0x340b_c6d9);
        // An empty volume-table record's CRC, in every UBI layout volume.
        assert_eq!(ubi_crc(&[0u8; 168]), 0xf116_c36b);
        assert_eq!(update(update(!0, b"1234"), b"56789"), ubi_crc(b"123456789"));
    }
}
