//! The identity contract for a board too small for the Raze's stack (no
//! systemd, avahi, or web server; the Luckfox Pico Pro's BusyBox image): Atlas
//! finds it by mDNS (`_pd-device._tcp`) and reads
//! `GET :5899/.well-known/pd-device`, the same as any board.
//!
//! Read-only: it answers those two things and takes no commands.
//! Standard library only, so it costs the image a few hundred KB.

pub mod identity;
pub mod mdns;
pub mod server;

/// The identity endpoint's port and path, as every board serves them.
pub const PORT: u16 = 5899;
pub const PATH: &str = "/.well-known/pd-device";
