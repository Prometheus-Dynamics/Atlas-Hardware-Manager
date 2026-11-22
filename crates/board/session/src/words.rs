//! How long something has taken, in plain words for people: `400 ms` below a second, `4.8 s` below ten
//! seconds, `12 s` below a minute, then `1 min 5 s`, or `15 min` when there are no seconds left over.

use std::time::Duration;

/// A duration in plain words (see the module note).
#[must_use]
pub fn duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1_000 {
        return format!("{ms} ms");
    }
    if ms < 10_000 {
        let tenths = ms / 100;
        return if tenths.is_multiple_of(10) {
            format!("{} s", tenths / 10)
        } else {
            format!("{}.{} s", tenths / 10, tenths % 10)
        };
    }
    let seconds = d.as_secs();
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let (minutes, rest) = (seconds / 60, seconds % 60);
    if rest == 0 {
        format!("{minutes} min")
    } else {
        format!("{minutes} min {rest} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_for_each_span() {
        assert_eq!(duration(Duration::from_millis(0)), "0 ms");
        assert_eq!(duration(Duration::from_millis(400)), "400 ms");
        assert_eq!(duration(Duration::from_millis(4_800)), "4.8 s");
        assert_eq!(duration(Duration::from_secs(4)), "4 s");
        assert_eq!(duration(Duration::from_secs(12)), "12 s");
        assert_eq!(duration(Duration::from_secs(59)), "59 s");
        assert_eq!(duration(Duration::from_secs(65)), "1 min 5 s");
        assert_eq!(duration(Duration::from_secs(900)), "15 min");
    }
}
