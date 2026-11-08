//! The loader header's release time.

/// The header's release time (the C reads the build machine's local clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl ReleaseTime {
    /// UTC from Unix seconds (the C uses `localtime`; the caller picks).
    pub fn from_unix_utc(seconds: u64) -> Self {
        let (days, rest) = (seconds / 86_400, seconds % 86_400);
        // Howard Hinnant's civil_from_days.
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: year as u16,
            month,
            day,
            hour: (rest / 3_600) as u8,
            minute: (rest % 3_600 / 60) as u8,
            second: (rest % 60) as u8,
        }
    }
}
