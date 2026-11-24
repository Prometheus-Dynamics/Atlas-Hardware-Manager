//! Every integration test of this crate in one binary: one link instead of 7. Each file is
//! a module; nextest still runs every test in its own process.

mod common;
mod lease;
mod lemnos_lookup;
mod long;
mod names;
mod pin;
mod session;
mod ways_in;
