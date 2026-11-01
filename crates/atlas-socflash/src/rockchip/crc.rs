//! The two CRCs Rockchip's tools use (rkdeveloptool crc.cpp).

/// CRC-16/CCITT-FALSE: polynomial 0x1021, MSB first, from 0xFFFF, no final
/// XOR (`CRC_CCITT`). Appended big-endian to every maskrom stage.
pub fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// rkdeveloptool's `CRC_32`: MSB first, from 0, no final XOR, over
/// `polynomial`. Its table says 0x04C10DB7, one bit off the usual
/// 0x04C11DB7, and real loaders use it: rkbin's
/// rk3588_spl_loader_v1.18.113.bin and rv1106_download_v1.15.108.bin
/// (Gaia's builds, 2026-10-31). The loader check still accepts the standard
/// one second ([`LOADER_POLYNOMIALS`]), for boot_merger builds not yet seen.
pub fn rk_crc32(bytes: &[u8], polynomial: u32) -> u32 {
    let mut crc: u32 = 0;
    for byte in bytes {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ polynomial
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The polynomial in rkdeveloptool's table first, the standard one second.
pub const LOADER_POLYNOMIALS: [u32; 2] = [0x04c1_0db7, 0x04c1_1db7];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values() {
        assert_eq!(crc16_ccitt(b"123456789"), 0x29b1);
        assert_eq!(crc16_ccitt(b""), 0xffff);
        // The research spec's value for rkdeveloptool's polynomial; the
        // standard one gives CRC-32/MPEG-2 without its 0xFFFFFFFF start.
        assert_eq!(rk_crc32(b"123456789", 0x04c1_0db7), 0x889a_9615);
        assert_eq!(rk_crc32(&[1], 0x04c1_0db7), 0x04c1_0db7);
    }
}
