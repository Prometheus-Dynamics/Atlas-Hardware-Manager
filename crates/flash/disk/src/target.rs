//! Where an image is written: a disk that can go away and come back.

use std::io;
use std::time::Duration;

/// A disk an image is written to. A drop, a failed read-back or a failed
/// reattach is handled by the writer; the target only reports what happened.
pub trait Target {
    /// Bytes the target holds.
    fn capacity(&self) -> u64;

    /// Names this target for a journal. A later run trusts a journal only for
    /// the same target, so two disks never share progress.
    fn identity(&self) -> String;

    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()>;

    /// Makes what was written durable on the medium.
    fn flush(&mut self) -> io::Result<()>;

    /// Reads from the medium, not from a cache.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;

    /// After a drop: one attempt to find and reopen the disk. `Err` says why
    /// it did not work this time.
    fn reattach(&mut self) -> Result<(), String>;

    /// Waits before the next reattach attempt. A target that cannot wait may
    /// return at once.
    fn pause(&mut self, how_long: Duration) {
        std::thread::sleep(how_long);
    }
}
