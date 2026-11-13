//! The failure rule's delay: never give up, and never wait longer than the cap.

use std::time::Duration;

/// The first delay after a failure.
pub const FIRST: Duration = Duration::from_millis(250);
/// The longest delay between two tries. The rule allows 5 to 10 seconds.
pub const CAP: Duration = Duration::from_secs(5);

/// Doubling delays from [`FIRST`] up to [`CAP`], restarted by a success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: FIRST }
    }
}

impl Backoff {
    /// The delay before the next try. Each call doubles the one after it, up to the cap.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(CAP);
        delay
    }

    /// Starts again from the first delay, after a success.
    pub fn reset(&mut self) {
        self.next = FIRST;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_double_from_the_first_and_stop_at_the_cap() {
        let mut backoff = Backoff::default();
        let delays: Vec<u64> = (0..8)
            .map(|_| backoff.next_delay().as_millis() as u64)
            .collect();
        assert_eq!(delays, [250, 500, 1000, 2000, 4000, 5000, 5000, 5000]);
    }

    #[test]
    fn a_success_starts_again_from_the_first_delay() {
        let mut backoff = Backoff::default();
        for _ in 0..6 {
            backoff.next_delay();
        }
        backoff.reset();
        assert_eq!(backoff.next_delay(), FIRST);
    }
}
