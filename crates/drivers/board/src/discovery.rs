//! Where a scan looks for Orion nodes. This is the one function a scan calls.
//!
//! Orion's discovery is in its pinned tree (Orion b01a7fb2, crates/comm/discovery: `mdns.rs` browses
//! `_orion._tcp.local.` and reports the add and remove events as `PeerEvent`). The ticket name "D12" is
//! not in that tree. Atlas does not read Orion's discovery yet, so the caller supplies the addresses;
//! this function is where that read goes, and it keeps its place in the driver. Discovery proves
//! nothing: a node found here is only offered after it answers Identify (see [`crate::Presence`]).

use atlas_types::Link;
use orion::Address;

use atlas_session::Seed;

/// The Orion addresses that a scan of `link` should try, from the seeds the caller gave.
pub fn addresses_on(seeds: &[Seed], link: &Link) -> Vec<Address> {
    seeds
        .iter()
        .filter(|seed| seed.is_on(link))
        .map(|seed| seed.address.clone())
        .collect()
}
