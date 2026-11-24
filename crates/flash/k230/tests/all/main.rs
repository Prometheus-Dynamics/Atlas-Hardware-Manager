//! Every integration test of this crate in one binary: one link instead of 4. Each file is
//! a module; nextest still runs every test in its own process.

mod bootrom;
mod burner;
mod common;
mod vectors;
mod write;
