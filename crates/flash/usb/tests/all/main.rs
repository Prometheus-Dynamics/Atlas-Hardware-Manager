//! Every integration test of this crate in one binary: one link instead of 3. Each file is
//! a module; nextest still runs every test in its own process.

mod interface_string;
mod link;
mod ports;
