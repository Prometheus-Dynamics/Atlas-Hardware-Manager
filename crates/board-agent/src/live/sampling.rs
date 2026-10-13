//! When a series is sampled: `BOARD_AGENT_LIVE_SAMPLING` for the board,
//! `BOARD_AGENT_LIVE_SAMPLING_SERIES` per stream or series.

/// A sampling mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Always, so a page opened later still gets the history.
    Always,
    /// Always for what costs next to nothing to read (/sys, /proc), only
    /// while someone watches for the rest (lemnosd's devices).
    Cheap,
    /// Only while someone watches.
    Subscribed,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "always" => Some(Self::Always),
            "cheap" => Some(Self::Cheap),
            "subscribed" => Some(Self::Subscribed),
            _ => None,
        }
    }
}

/// The board's mode and the overrides, first match wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub board: Mode,
    /// (pattern, mode): a pattern is a series name or a prefix ending in
    /// `*`, matched against the series name and against `<stream>/<series>`
    /// (so `board.hardware/*` names a whole stream).
    pub series: Vec<(String, Mode)>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            board: Mode::Cheap,
            series: Vec::new(),
        }
    }
}

impl Rules {
    /// `pattern=mode,pattern=mode`.
    pub fn parse_series(raw: &str) -> Result<Vec<(String, Mode)>, String> {
        raw.split(',')
            .map(str::trim)
            .filter(|rule| !rule.is_empty())
            .map(|rule| {
                let (pattern, mode) = rule
                    .split_once('=')
                    .ok_or_else(|| format!("`{rule}`: want pattern=mode"))?;
                let pattern = pattern.trim();
                let mode =
                    Mode::parse(mode).ok_or_else(|| format!("`{rule}`: unknown mode `{mode}`"))?;
                if pattern.is_empty() || pattern.trim_end_matches('*').contains('*') {
                    return Err(format!("`{rule}`: a name, or a prefix ending in `*`"));
                }
                Ok((pattern.to_owned(), mode))
            })
            .collect()
    }

    /// Whether `series` of `stream` is sampled always (else only while
    /// someone watches). `cheap`: reading it costs next to nothing.
    pub fn always(&self, stream: &str, series: &str, cheap: bool) -> bool {
        let qualified = format!("{stream}/{series}");
        let matches = |pattern: &str| match pattern.strip_suffix('*') {
            Some(prefix) => series.starts_with(prefix) || qualified.starts_with(prefix),
            None => pattern == series || pattern == qualified,
        };
        let mode = self
            .series
            .iter()
            .find(|(pattern, _)| matches(pattern))
            .map_or(self.board, |(_, mode)| *mode);
        match mode {
            Mode::Always => true,
            Mode::Cheap => cheap,
            Mode::Subscribed => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_matching_rule_decides_and_cheap_follows_the_source() {
        let rules = Rules {
            board: Mode::Cheap,
            series: Rules::parse_series(" imu.* = always, board.live/cpu.core*=subscribed ")
                .expect("parses"),
        };
        assert!(rules.always("board.live", "cpu", true));
        assert!(!rules.always("board.live", "cpu.core2", true));
        assert!(rules.always("board.hardware", "imu.accel_x", false));
        assert!(!rules.always("board.hardware", "power.voltage", false));
        let rules = Rules {
            board: Mode::Subscribed,
            series: Rules::parse_series("board.hardware/*=always").expect("parses"),
        };
        assert!(!rules.always("board.live", "cpu", true));
        assert!(rules.always("board.hardware", "fan.rpm", false));
        for bad in ["cpu", "cpu=sometimes", "c*u=always", "=always"] {
            assert!(Rules::parse_series(bad).is_err(), "{bad}");
        }
    }
}
