//! The shared vocabulary of Atlas: who a device is, how it is reached, what
//! state it is in, and what it can do.
//!
//! Everything above the drivers works with these types only. No type here
//! names a device family, a board or a product.

mod capability;
mod device;
mod ids;
mod link;

pub use capability::{CapabilityKind, CapabilitySet, Offers, Reading};
pub use device::{DeviceState, Identity, LemnosState};
pub use ids::{DeviceKey, Family, KeyError, LinkId, NoBoardIdentity, Serial};
pub use link::{Candidate, Link, LinkKind};
