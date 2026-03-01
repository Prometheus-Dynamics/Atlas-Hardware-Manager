use super::*;

pub(crate) fn parse_verification_mismatch_offset(message: &str) -> Option<u64> {
    let marker = "Verification failed at byte offset ";
    let start = message.find(marker)? + marker.len();
    let digits = message[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

pub(crate) fn root_partition_span_from_image(image_path: &Path) -> Result<Option<(u64, u64)>, String> {
    let root_range = read_mbr_partition_spans(image_path)?
        .and_then(|partitions| {
            partitions
                .iter()
                .find(|partition| partition.part_type == 0x83)
                .or_else(|| partitions.get(1))
                .map(|partition| {
                    (
                        partition.start_lba.saturating_mul(512),
                        partition.sectors.saturating_mul(512),
                    )
                })
        })
        .filter(|(_, length)| *length > 0);
    Ok(root_range)
}

pub(crate) fn mismatch_offset_in_partition(
    mismatch_offset: Option<u64>,
    partition_span: Option<(u64, u64)>,
) -> bool {
    mismatch_offset
        .zip(partition_span)
        .map(|(offset, (partition_start, partition_len))| {
            offset >= partition_start && offset < partition_start.saturating_add(partition_len)
        })
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
pub(crate) fn strict_flash_verify_enabled() -> bool {
    env::var("ATLAS_STRICT_FLASH_VERIFY")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn strict_flash_verify_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verification_mismatch_offset() {
        let message =
            "Verification failed at byte offset 1206911515: flashed data does not match source image.";
        assert_eq!(
            parse_verification_mismatch_offset(message),
            Some(1_206_911_515)
        );
    }

    #[test]
    fn detects_offset_inside_partition() {
        assert!(mismatch_offset_in_partition(
            Some(1_206_911_515),
            Some((1_000_000_000, 800_000_000))
        ));
    }

    #[test]
    fn detects_offset_outside_partition() {
        assert!(!mismatch_offset_in_partition(
            Some(900_000_000),
            Some((1_000_000_000, 800_000_000))
        ));
    }
}
