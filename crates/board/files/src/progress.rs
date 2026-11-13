//! What a push reports: its stage, the bytes the board holds, and the rate.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far back the rate looks.
const WINDOW: Duration = Duration::from_secs(5);
/// Below this span a rate is not meaningful yet, and reads as zero.
const MIN_SPAN: Duration = Duration::from_millis(250);

/// Where a push is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Submitted; not started yet.
    Queued,
    /// Bytes are moving to the board.
    Sending,
    /// The session to the node is gone. The push waits for it and goes on by itself, with a
    /// wait that never exceeds the failure rule's cap. This is not an error: nothing has failed.
    WaitingForBoard {
        /// Why the session is not there, as the monitor last saw it.
        reason: String,
    },
    /// The board holds the whole file and its own check of the digest passed.
    Committed,
    /// The push stopped for good, with the reason. Only a changed source, a file the board
    /// refuses, or a source that cannot be read ends a push this way.
    Failed {
        /// Why, in words.
        reason: String,
    },
    /// Stopped by a person. The bytes the board already holds stay there; pushing the same
    /// file again resumes from them.
    Cancelled,
}

/// A push's state as it last changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Where the push is.
    pub stage: Stage,
    /// Bytes the board holds, in order from the start: what it has acknowledged.
    pub sent: u64,
    /// Bytes in the whole file, as read when the push started.
    pub total: u64,
    /// Bytes per second the board acknowledged over the last few seconds. Zero when nothing
    /// is moving.
    pub rate_bytes_per_sec: u64,
}

impl Progress {
    /// Nothing sent yet, with the total known.
    #[must_use]
    pub const fn queued(total: u64) -> Self {
        Self {
            stage: Stage::Queued,
            sent: 0,
            total,
            rate_bytes_per_sec: 0,
        }
    }
}

/// The rate from timed samples of the board's acknowledged bytes.
#[derive(Debug, Default)]
pub(crate) struct Meter {
    samples: VecDeque<(Instant, u64)>,
}

impl Meter {
    /// Records that the board holds `bytes` now, and returns the rate over the window.
    pub(crate) fn record(&mut self, at: Instant, bytes: u64) -> u64 {
        self.samples.push_back((at, bytes));
        while self.samples.len() > 2 {
            match self.samples.front() {
                Some((first, _)) if at.duration_since(*first) > WINDOW => {
                    self.samples.pop_front();
                }
                _ => break,
            }
        }
        match (self.samples.front(), self.samples.back()) {
            (Some(&(t0, b0)), Some(&(t1, b1))) if t1.duration_since(t0) >= MIN_SPAN => {
                let secs = t1.duration_since(t0).as_secs_f64();
                (b1.saturating_sub(b0) as f64 / secs) as u64
            }
            _ => 0,
        }
    }

    /// Forgets the samples: the board is not moving bytes now.
    pub(crate) fn clear(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_is_bytes_over_the_span_and_zero_before_it_is_meaningful() {
        let mut meter = Meter::default();
        let start = Instant::now();
        assert_eq!(meter.record(start, 0), 0);
        assert_eq!(meter.record(start + Duration::from_millis(100), 1000), 0);
        // 2000 bytes over 1 s.
        assert_eq!(meter.record(start + Duration::from_secs(1), 2000), 2000);
    }

    #[test]
    fn samples_older_than_the_window_are_dropped() {
        let mut meter = Meter::default();
        let start = Instant::now();
        meter.record(start, 0);
        meter.record(start + Duration::from_secs(1), 1_000);
        // Ten seconds later only the last two samples count.
        let rate = meter.record(start + Duration::from_secs(11), 11_000);
        assert_eq!(rate, 1_000);
        assert!(meter.samples.len() <= 2);
    }
}
