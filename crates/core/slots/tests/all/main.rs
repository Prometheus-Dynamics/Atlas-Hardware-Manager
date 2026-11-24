//! Every integration test of this crate in one binary: one link instead of 2. Each file is
//! a module; nextest still runs every test in its own process.

mod list;
mod policy;
