//! Every integration test of this crate in one binary: one link instead of 6. Each file is
//! a module; nextest still runs every test in its own process.

mod bundle;
mod image;
mod lemnos_fixture;
mod seed;
mod vectors;
mod veritysetup;
