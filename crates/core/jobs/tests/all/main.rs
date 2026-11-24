//! Every integration test of this crate in one binary: one link instead of 4. Each file is
//! a module; nextest still runs every test in its own process.

mod any_family;
mod board_identity;
mod jobs;
mod scan;
