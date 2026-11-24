//! Every integration test of this crate in one binary: one link instead of 3. Each file is
//! a module; nextest still runs every test in its own process.

mod any_family;
mod contract;
mod driver_isolation;
