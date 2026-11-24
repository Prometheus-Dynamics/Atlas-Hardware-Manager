//! Every integration test of this crate in one binary: one link instead of 5. Each file is
//! a module; nextest still runs every test in its own process.

mod bridge_surface;
mod identity_facts;
mod image;
mod sim_node;
mod trial;
