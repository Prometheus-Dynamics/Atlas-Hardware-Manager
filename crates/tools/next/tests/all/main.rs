//! Every integration test of this crate in one binary: one link instead of 8. Each file is
//! a module; nextest still runs every test in its own process.

mod bench_key;
mod common;
mod cycle;
mod identity;
mod offers;
mod security;
mod slots;
mod time;
mod usb_boot;
