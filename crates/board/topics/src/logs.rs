//! The node's log lines, followed and kept in a ring.

use std::collections::VecDeque;

/// The topic a node publishes its log lines on. Atlas's name for it: the simulated node uses the
/// same one, and the Lemnos bridge does not publish logs yet.
pub const LOG_TOPIC: &str = "logs";

/// How many log lines Atlas keeps. A node keeps 64 lines on its log topic, so a new session's
/// replay of them fits with room for a few hundred lines that arrive while the app is open.
pub const LOG_CAPACITY: usize = 256;

/// One line of a node's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The time the node gave the line, in its own clock.
    pub time: u64,
    /// The line as written.
    pub text: String,
}

/// The newest [`LOG_CAPACITY`] lines. Pushing past the capacity drops the oldest line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogRing {
    lines: VecDeque<LogLine>,
}

impl LogRing {
    /// Adds a line as the newest, dropping the oldest when the ring is full.
    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == LOG_CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// Whether the ring holds this line: the same time and text. A replay of the node's log on a
    /// new session carries lines the ring already has, and this is how they are told apart.
    #[must_use]
    pub fn contains(&self, line: &LogLine) -> bool {
        self.lines.iter().any(|held| held == line)
    }

    /// The lines, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &LogLine> {
        self.lines.iter()
    }

    /// How many lines are kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether no line is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(time: u64) -> LogLine {
        LogLine {
            time,
            text: format!("line {time}"),
        }
    }

    #[test]
    fn the_ring_keeps_its_capacity_and_the_newest_lines() {
        let mut ring = LogRing::default();
        for time in 0..(LOG_CAPACITY as u64 + 100) {
            ring.push(line(time));
        }
        assert_eq!(ring.len(), LOG_CAPACITY);
        assert_eq!(ring.iter().next().map(|line| line.time), Some(100));
        assert_eq!(
            ring.iter().last().map(|line| line.time),
            Some(LOG_CAPACITY as u64 + 99)
        );
    }

    #[test]
    fn an_empty_ring_has_no_newest_time() {
        let ring = LogRing::default();
        assert!(ring.is_empty());
        assert_eq!(ring.iter().next(), None);
    }
}
