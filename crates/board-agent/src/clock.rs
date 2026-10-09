//! `clock.set`: sets the board's clock (no RTC battery, often no NTP) to a
//! time the caller sends, the Orion form of Atlas's SSH "Set clock".
//!
//! Args: `unix` (`Int` or `UInt`, seconds since 1970, UTC) or `time`
//! (`String`, `YYYY-MM-DDThh:mm:ssZ`; a missing `Z` means UTC too). Times
//! before 2024 or from 2100 on are refused: a caller with a broken clock of
//! its own must not drag the board's with it.

use std::collections::BTreeMap;

use orion_control_plane::TypedConfigValue;

/// The action name the agent claims.
pub const CLOCK_SET: &str = "clock.set";

/// 2024-01-01T00:00:00Z: nothing this agent runs on predates it.
const EARLIEST: i64 = 1_704_067_200;
/// 2100-01-01T00:00:00Z.
const LATEST: i64 = 4_102_444_800;

/// The requested time, in Unix seconds.
pub fn requested_time(args: &BTreeMap<String, TypedConfigValue>) -> Result<i64, String> {
    let secs = match (args.get("unix"), args.get("time")) {
        (Some(TypedConfigValue::Int(secs)), None) => *secs,
        (Some(TypedConfigValue::UInt(secs)), None) => {
            i64::try_from(*secs).map_err(|_| "`unix` is out of range".to_owned())?
        }
        (None, Some(TypedConfigValue::String(text))) => parse_utc(text)?,
        (Some(_), Some(_)) => return Err("send `unix` or `time`, not both".into()),
        (Some(_), None) => return Err("`unix` must be an integer (seconds since 1970)".into()),
        (None, Some(_)) => return Err("`time` must be a string (YYYY-MM-DDThh:mm:ssZ)".into()),
        (None, None) => return Err("`unix` (seconds since 1970) or `time` is required".into()),
    };
    if !(EARLIEST..LATEST).contains(&secs) {
        return Err(format!(
            "{secs} is not a plausible time (2024 to 2099); is the sender's clock right?"
        ));
    }
    Ok(secs)
}

/// `YYYY-MM-DDThh:mm:ss[Z]` (a space for the `T` works too), UTC only.
fn parse_utc(text: &str) -> Result<i64, String> {
    let bad = || format!("`time` must look like 2026-10-09T12:00:00Z, got `{text}`");
    let text = text.trim();
    let text = text.strip_suffix('Z').unwrap_or(text);
    let (date, clock) = text.split_once(['T', ' ']).ok_or_else(bad)?;
    let mut date = date.split('-').map(str::parse::<i64>);
    let mut clock = clock.split(':').map(str::parse::<i64>);
    let next = |part: &mut dyn Iterator<Item = Result<i64, _>>| -> Result<i64, String> {
        part.next().and_then(Result::ok).ok_or_else(bad)
    };
    let (year, month, day) = (next(&mut date)?, next(&mut date)?, next(&mut date)?);
    let (hour, minute, second) = (next(&mut clock)?, next(&mut clock)?, next(&mut clock)?);
    if date.next().is_some()
        || clock.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..61).contains(&second)
    {
        return Err(bad());
    }
    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since 1970 now, by this board's clock.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(key: &str, value: TypedConfigValue) -> BTreeMap<String, TypedConfigValue> {
        BTreeMap::from([(key.to_owned(), value)])
    }

    #[test]
    fn unix_and_iso_times_are_accepted() {
        let t = 1_791_590_400; // 2026-10-10T00:00:00Z
        assert_eq!(
            requested_time(&args("unix", TypedConfigValue::Int(t))),
            Ok(t)
        );
        assert_eq!(
            requested_time(&args("unix", TypedConfigValue::UInt(t as u64))),
            Ok(t)
        );
        for text in [
            "2026-10-10T00:00:00Z",
            "2026-10-10 00:00:00",
            "2026-10-10T00:00:00",
        ] {
            assert_eq!(
                requested_time(&args("time", TypedConfigValue::String(text.into()))),
                Ok(t),
                "{text}"
            );
        }
        assert_eq!(
            requested_time(&args(
                "time",
                TypedConfigValue::String("2024-02-29T12:34:56Z".into())
            )),
            Ok(1_709_210_096)
        );
    }

    #[test]
    fn implausible_or_malformed_times_are_refused() {
        assert!(requested_time(&BTreeMap::new()).is_err());
        assert!(requested_time(&args("unix", TypedConfigValue::Int(0))).is_err());
        assert!(requested_time(&args("unix", TypedConfigValue::Int(LATEST))).is_err());
        assert!(
            requested_time(&args("unix", TypedConfigValue::String("1791590400".into()))).is_err()
        );
        for text in [
            "yesterday",
            "2026-13-01T00:00:00Z",
            "2026-10-10T25:00:00Z",
            "2026-10-10T00:00:00+02:00",
        ] {
            assert!(
                requested_time(&args("time", TypedConfigValue::String(text.into()))).is_err(),
                "{text}"
            );
        }
        let mut both = args("unix", TypedConfigValue::Int(1_791_590_400));
        both.insert(
            "time".into(),
            TypedConfigValue::String("2026-10-10T00:00:00Z".into()),
        );
        assert!(requested_time(&both).is_err());
    }
}
