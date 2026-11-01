//! RC4 with Rockchip's fixed key (U-Boot tools/rkcommon.c `rc4_key`),
//! which scrambles idblock headers: a loader's FlashHead is the RKNS header
//! scrambled this way, each 512-byte block from a fresh key schedule.

pub const KEY: [u8; 16] = [124, 78, 3, 4, 85, 5, 9, 7, 45, 44, 123, 56, 23, 13, 23, 17];

/// XORs `data` with the keystream from a fresh schedule (its own inverse).
pub fn rc4(data: &[u8]) -> Vec<u8> {
    let mut state: [u8; 256] = std::array::from_fn(|index| index as u8);
    let mut j: u8 = 0;
    for i in 0..256 {
        j = j.wrapping_add(state[i]).wrapping_add(KEY[i % KEY.len()]);
        state.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(state[i as usize]);
            state.swap(i as usize, j as usize);
            byte ^ state[state[i as usize].wrapping_add(state[j as usize]) as usize]
        })
        .collect()
}

/// `data` unscrambled in 512-byte blocks, as rkcommon writes it.
pub fn rc4_blocks(data: &[u8]) -> Vec<u8> {
    data.chunks(512).flat_map(rc4).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keystream_matches_the_research_vector() {
        let stream = rc4(&[0u8; 16]);
        assert_eq!(
            stream,
            [
                0x6e, 0x26, 0x2c, 0xf3, 0xbe, 0x9f, 0x9d, 0x51, 0xea, 0x30, 0x34, 0xce, 0x20, 0x51,
                0x1f, 0x98
            ]
        );
        // A real FlashHead's first bytes (rkbin's RK3588 and RV1106 loaders)
        // unscramble to the RKNS magic.
        assert_eq!(&rc4(&[0x3c, 0x6d, 0x62, 0xa0]), b"RKNS");
    }
}
