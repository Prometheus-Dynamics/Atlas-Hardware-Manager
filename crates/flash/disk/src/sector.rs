//! Sector-aligned I/O for raw disks that take whole sectors only (Windows and
//! macOS raw devices). Each call starts on a sector boundary. The whole sectors
//! go out as they are. A final part that ends inside a sector is finished by
//! reading that sector, changing its first bytes, and writing the sector back:
//! the bytes after the image keep the values the medium already held.
//!
//! Pure logic over a [`Positional`] device, so the rules are tested here on
//! Linux with an in-memory disk.

use std::io;

use crate::platform::RawDisk;

/// Positioned I/O on a raw device.
pub(crate) trait Positional: Send {
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> io::Result<()>;
    fn pread(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    fn fsync(&mut self) -> io::Result<()>;
}

/// A raw device that takes whole sectors. `sector` is the logical sector size
/// the system reports for the disk (512 or 4096).
#[derive(Debug)]
pub(crate) struct Sectored<T> {
    inner: T,
    sector: usize,
}

impl<T> Sectored<T> {
    pub(crate) fn new(inner: T, sector: usize) -> io::Result<Self> {
        if sector == 0 || !sector.is_power_of_two() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("a sector size of {sector} bytes is not a power of two"),
            ));
        }
        Ok(Self { inner, sector })
    }

    fn check_offset(&self, offset: u64) -> io::Result<()> {
        if offset.is_multiple_of(self.sector as u64) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "the raw write at byte {offset} does not start on a {}-byte sector",
                    self.sector
                ),
            ))
        }
    }
}

impl<T: Positional> RawDisk for Sectored<T> {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.check_offset(offset)?;
        let whole = data.len() - data.len() % self.sector;
        if whole > 0 {
            self.inner.pwrite(offset, &data[..whole])?;
        }
        let rest = &data[whole..];
        if !rest.is_empty() {
            let at = offset + whole as u64;
            let mut sector_bytes = vec![0u8; self.sector];
            self.inner.pread(at, &mut sector_bytes)?;
            sector_bytes[..rest.len()].copy_from_slice(rest);
            self.inner.pwrite(at, &sector_bytes)?;
        }
        Ok(())
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.check_offset(offset)?;
        let whole = buf.len() - buf.len() % self.sector;
        if whole > 0 {
            self.inner.pread(offset, &mut buf[..whole])?;
        }
        let rest_len = buf.len() - whole;
        if rest_len > 0 {
            let mut sector_bytes = vec![0u8; self.sector];
            self.inner.pread(offset + whole as u64, &mut sector_bytes)?;
            buf[whole..].copy_from_slice(&sector_bytes[..rest_len]);
        }
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        self.inner.fsync()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory disk. Counts flushes so tests can see them.
    struct Memory {
        bytes: Vec<u8>,
        syncs: u32,
    }

    impl Positional for Memory {
        fn pwrite(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
            let start = offset as usize;
            self.bytes[start..start + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn pread(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            let start = offset as usize;
            buf.copy_from_slice(&self.bytes[start..start + buf.len()]);
            Ok(())
        }
        fn fsync(&mut self) -> io::Result<()> {
            self.syncs += 1;
            Ok(())
        }
    }

    fn disk(size: usize, fill: u8) -> Sectored<Memory> {
        Sectored::new(
            Memory {
                bytes: vec![fill; size],
                syncs: 0,
            },
            512,
        )
        .unwrap()
    }

    #[test]
    fn a_tail_inside_a_sector_keeps_the_bytes_after_it() {
        let mut device = disk(4096, 0xAA);
        device.write_at(0, &[1u8; 1000]).unwrap();
        let bytes = &device.inner.bytes;
        assert!(bytes[..1000].iter().all(|b| *b == 1));
        assert!(
            bytes[1000..1024].iter().all(|b| *b == 0xAA),
            "the rest of the sector the image ends in is unchanged"
        );
        assert!(bytes[1024..].iter().all(|b| *b == 0xAA));
    }

    #[test]
    fn whole_sectors_are_written_in_one_call_and_read_back_with_a_tail() {
        let mut device = disk(4096, 0);
        let data: Vec<u8> = (0..1300u32).map(|n| (n % 251) as u8).collect();
        device.write_at(512, &data).unwrap();
        let mut back = vec![0u8; 1300];
        device.read_at(512, &mut back).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn a_write_that_does_not_start_on_a_sector_is_refused_before_the_device() {
        let mut device = disk(4096, 0);
        let error = device.write_at(100, &[1u8; 512]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(device.inner.bytes.iter().all(|b| *b == 0));
    }

    #[test]
    fn a_sync_reaches_the_device() {
        let mut device = disk(1024, 0);
        device.sync().unwrap();
        device.sync().unwrap();
        assert_eq!(device.inner.syncs, 2);
    }

    #[test]
    fn a_sector_size_must_be_a_power_of_two() {
        let memory = || Memory {
            bytes: vec![0; 16],
            syncs: 0,
        };
        assert!(Sectored::new(memory(), 0).is_err());
        assert!(Sectored::new(memory(), 768).is_err());
        assert!(Sectored::new(memory(), 4096).is_ok());
    }
}
