//! The caps on what is copied out of the image: one file, and the boot files in
//! total. Checked before a byte is copied, so a huge declared size is refused
//! rather than written.

use crate::error::ImageError;

#[derive(Clone, Debug)]
pub(crate) struct Budget {
    left: u64,
    per_file: u64,
}

impl Budget {
    pub(crate) fn new(total: u64, per_file: u64) -> Self {
        Self {
            left: total,
            per_file,
        }
    }

    pub(crate) fn take(&mut self, size: u64) -> Result<(), ImageError> {
        if size > self.per_file {
            return Err(ImageError::Limit(format!(
                "a boot file is {size} bytes, more than the {} allowed",
                self.per_file
            )));
        }
        if size > self.left {
            return Err(ImageError::Limit(format!(
                "the boot files would pass the {} byte total",
                self.left
            )));
        }
        self.left -= size;
        Ok(())
    }
}
