//! Where a scan looks for Orion nodes. This is the one function a scan calls.
//!
//! Until Orion's discovery lands (Orion D12: add and remove events for `_orion._tcp`, and the
//! local serial ports as `serial:` addresses), the caller supplies the addresses. D12 replaces
//! this function's body and keeps its place in the driver. Discovery proves nothing: a node
//! found here is only offered after it answers Identify (see [`crate::Presence`]).

use atlas_types::Link;
use orion::Address;

use crate::reach::Seed;

/// The Orion addresses that a scan of `link` should try, from the seeds the caller gave.
pub fn addresses_on(seeds: &[Seed], link: &Link) -> Vec<Address> {
    seeds
        .iter()
        .filter(|seed| seed.is_on(link))
        .map(|seed| seed.address.clone())
        .collect()
}
