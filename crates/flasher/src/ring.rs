//! The status ring while flashing, and what the eMMC holds: the board shows
//! how far it is with no host UI, and the daemon knows whether rebooting to
//! the eMMC is safe.
//!
//! Frames go straight to the ring's character device (`/dev/leds0`, 4 bytes
//! per LED: R, G, B, W, as raze-leds writes them): no lemnosd in the
//! flasher. Logical LED 0 is the ring's top (physical `offset`), running
//! with the physical indices (`clockwise`, Lemnos's sense).

use std::path::PathBuf;
use std::sync::Mutex;

/// What the ring shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// Up, waiting for Atlas (dim green).
    Waiting,
    /// Writing the image: an amber arc, 0..1.
    Writing(f32),
    /// Reading it back: a blue arc, 0..1.
    Verifying(f32),
    /// Written and verified (bright green).
    Done,
    /// Something failed (red); the board stays in the flasher.
    Failed,
}

/// Something that shows a [`Look`].
pub trait Show: Send + Sync {
    fn show(&self, look: Look);
}

/// No ring (tests, a board without one).
pub struct NoRing;

impl Show for NoRing {
    fn show(&self, _look: Look) {}
}

/// The status ring at `path`.
pub struct Ring {
    pub path: PathBuf,
    pub count: usize,
    pub offset: usize,
    pub clockwise: bool,
    /// The last frame written, so an unchanged look writes nothing.
    last: Mutex<Vec<u8>>,
}

impl Ring {
    /// The Raze's ring: 16 LEDs, logical 0 at physical 5, clockwise.
    pub fn raze() -> Self {
        Self::new("/dev/leds0".into(), 16, 5, true)
    }

    pub fn new(path: PathBuf, count: usize, offset: usize, clockwise: bool) -> Self {
        Self {
            path,
            count,
            offset,
            clockwise,
            last: Mutex::new(Vec::new()),
        }
    }

    /// The frame for `look`, in physical order.
    pub fn frame(&self, look: Look) -> Vec<u8> {
        const DIM: [u8; 3] = [4, 4, 6];
        let arc = |fraction: f32, color: [u8; 3]| -> Vec<[u8; 3]> {
            // Rounded down, but a started job shows at least one LED.
            let lit = ((fraction.clamp(0.0, 1.0) * self.count as f32) as usize)
                .max(usize::from(fraction > 0.0));
            (0..self.count)
                .map(|i| if i < lit { color } else { DIM })
                .collect()
        };
        let logical: Vec<[u8; 3]> = match look {
            Look::Waiting => vec![[0, 24, 0]; self.count],
            Look::Writing(fraction) => arc(fraction, [72, 36, 0]),
            Look::Verifying(fraction) => arc(fraction, [0, 28, 72]),
            Look::Done => vec![[0, 96, 0]; self.count],
            Look::Failed => vec![[72, 0, 0]; self.count],
        };
        let mut physical = vec![[0u8; 3]; self.count];
        for (i, color) in logical.into_iter().enumerate() {
            let at = if self.clockwise {
                (self.offset + i) % self.count
            } else {
                (self.offset + self.count - i % self.count) % self.count
            };
            physical[at] = color;
        }
        physical
            .into_iter()
            .flat_map(|[r, g, b]| [r, g, b, 0])
            .collect()
    }
}

impl Show for Ring {
    fn show(&self, look: Look) {
        let frame = self.frame(look);
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *last == frame {
            return;
        }
        // No ring (or not yet): the flash goes on without it.
        if std::fs::write(&self.path, &frame).is_ok() {
            *last = frame;
        }
    }
}

/// What the eMMC holds, as far as this flasher knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Emmc {
    /// Not touched this boot: rebooting to it is safe.
    #[default]
    Untouched,
    /// A write started and didn't finish well: it may not boot.
    Partial,
    /// Written, not verified yet.
    Written,
    /// Written and read back with the right SHA-256.
    Verified,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn led(frame: &[u8], physical: usize) -> [u8; 3] {
        [
            frame[physical * 4],
            frame[physical * 4 + 1],
            frame[physical * 4 + 2],
        ]
    }

    #[test]
    fn the_arc_starts_at_the_top_and_runs_with_the_physical_order() {
        let ring = Ring::new("/nonexistent".into(), 16, 5, true);
        let frame = ring.frame(Look::Writing(0.25));
        assert_eq!(frame.len(), 64, "16 LEDs, 4 bytes each");
        // Logical 0..4 are physical 5..9.
        for physical in 5..9 {
            assert_eq!(led(&frame, physical), [72, 36, 0], "LED {physical}");
        }
        assert_eq!(led(&frame, 9), [4, 4, 6]);
        assert_eq!(led(&frame, 4), [4, 4, 6]);
        // Counter-clockwise: logical 1 is physical 4.
        let ccw = Ring::new("/nonexistent".into(), 16, 5, false).frame(Look::Writing(0.125));
        assert_eq!(
            (led(&ccw, 5), led(&ccw, 4), led(&ccw, 6)),
            ([72, 36, 0], [72, 36, 0], [4, 4, 6])
        );
    }

    #[test]
    fn a_started_job_shows_at_least_one_led_and_the_end_looks_differ() {
        let ring = Ring::new("/nonexistent".into(), 16, 0, true);
        assert_eq!(led(&ring.frame(Look::Verifying(0.01)), 0), [0, 28, 72]);
        assert_eq!(led(&ring.frame(Look::Verifying(0.0)), 0), [4, 4, 6]);
        assert_eq!(led(&ring.frame(Look::Done), 7), [0, 96, 0]);
        assert_eq!(led(&ring.frame(Look::Failed), 7), [72, 0, 0]);
    }

    #[test]
    fn the_ring_device_gets_each_new_frame_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("leds0");
        let ring = Ring::new(path.clone(), 4, 0, true);
        ring.show(Look::Done);
        assert_eq!(std::fs::read(&path).unwrap(), [0, 96, 0, 0].repeat(4));
        std::fs::write(&path, b"x").unwrap();
        ring.show(Look::Done);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"x",
            "unchanged: not written again"
        );
    }
}
