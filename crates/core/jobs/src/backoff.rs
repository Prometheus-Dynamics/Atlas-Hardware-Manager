use std::time::Duration;

/// How long to wait before retry number `attempt`: doubling from `first`, never
/// more than `cap`. Retries never stop; only the wait is bounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub first: Duration,
    pub cap: Duration,
}

impl Backoff {
    /// The delay before retry `attempt`, counting from 1.
    pub fn delay(&self, attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1).min(31);
        self.first
            .checked_mul(1u32 << shift)
            .unwrap_or(self.cap)
            .min(self.cap)
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            first: Duration::from_millis(250),
            cap: Duration::from_secs(5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_then_holds_at_the_cap() {
        let backoff = Backoff::default();
        let delays: Vec<u64> = (1..=7)
            .map(|attempt| backoff.delay(attempt).as_millis() as u64)
            .collect();
        assert_eq!(delays, vec![250, 500, 1000, 2000, 4000, 5000, 5000]);
        assert_eq!(backoff.delay(u32::MAX), Duration::from_secs(5));
    }
}
