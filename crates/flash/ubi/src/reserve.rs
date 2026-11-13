//! The PEBs an attached image needs: the layout volume, UBI's own working
//! PEBs, every volume's reservation, and the bad-block reserve on a device.

use crate::headers::{LAYOUT_VOLUME_LEBS, RUNTIME_RESERVED_PEBS};

/// The PEBs an image needs before bad-block reserves: the layout volume,
/// UBI's two working PEBs and each volume's reserved LEBs.
pub fn reserved_pebs(volume_reserved_lebs: impl IntoIterator<Item = u32>) -> u64 {
    u64::from(LAYOUT_VOLUME_LEBS)
        + RUNTIME_RESERVED_PEBS
        + volume_reserved_lebs.into_iter().map(u64::from).sum::<u64>()
}

/// The bad-block reserve UBI keeps on a device of `device_pebs` PEBs
/// (`get_bad_peb_limit`): `per_1024` per 1024 PEBs, rounded up. With
/// `per_1024` 0 the kernel takes its figure from its config, which a file
/// cannot know; this counts 1 PEB then.
pub fn bad_peb_reserve(device_pebs: u64, per_1024: u64) -> u64 {
    let limit = device_pebs.saturating_mul(per_1024) / 1024;
    if limit.saturating_mul(1024) / per_1024.max(1) < device_pebs {
        limit + 1
    } else {
        limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_add_up() {
        assert_eq!(bad_peb_reserve(1024, 20), 20);
        assert_eq!(bad_peb_reserve(1000, 20), 20);
        assert_eq!(bad_peb_reserve(1025, 20), 21);
        assert_eq!(bad_peb_reserve(2048, 0), 1);
        // Huge counts saturate instead of overflowing.
        assert!(bad_peb_reserve(u64::MAX, 20) > 0);
        assert_eq!(reserved_pebs([4, 4, 1]), 2 + 2 + 9);
    }
}
