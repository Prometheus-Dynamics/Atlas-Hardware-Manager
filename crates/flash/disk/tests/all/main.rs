//! Every integration test of this crate in one binary: one link instead of 6. Each file is
//! a module; nextest still runs every test in its own process.

mod bounded_memory;
mod common;
mod multi_block;
mod resume;
mod seam;
mod write;
mod xz_stream;
