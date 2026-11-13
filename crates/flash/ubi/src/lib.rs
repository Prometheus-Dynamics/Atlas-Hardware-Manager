//! UBI images on the computer: reading and checking the images Gaia writes
//! for NAND boards, and the slot bundle they ship in.
//!
//! - [`image::read_image`] reads a whole image: every PEB's headers checked,
//!   the layout volume's two volume tables compared, each volume's LEBs and
//!   static data CRCs checked, and the reservations reported.
//! - [`headers`] are the on-flash headers (`drivers/mtd/ubi/ubi-media.h`):
//!   big-endian fields, each closed by its CRC. Decoding checks the magic,
//!   version, CRC and volume type; `validate` checks the fields the way the
//!   kernel does, as far as a file can tell.
//! - [`reserve`] counts the PEBs an image needs on a device.
//! - [`bundle`] reads the slot bundle (a plain ustar) member by member,
//!   without unpacking it.
//!
//! Every input is untrusted. Offsets and sizes are bounds-checked before use,
//! no header field sizes an allocation, and malformed bytes return an error.
//! This crate writes nothing and talks to no device: writing NAND slots on a
//! board is a Lemnos driver.

pub mod bundle;
pub mod crc;
pub mod headers;
pub mod image;
pub mod reserve;

#[cfg(test)]
mod test_images;
