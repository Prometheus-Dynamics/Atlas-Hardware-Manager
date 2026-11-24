//! Every integration test of this crate in one binary: one link instead of 6. Each file is
//! a module; nextest still runs every test in its own process.

mod blocks;
mod common;
mod flash;
mod forms;
mod hostile;
mod memory;
mod signing;
