//! atlas-core's integration tests, one module per area, built as a single
//! test binary: each binary links the whole crate graph, so seven separate
//! ones cost seven links per test run.

mod board_seq;
mod clock_sync;
mod core_flows;
mod history;
mod observe;
mod robots;
mod robustness;
mod selftest;
mod watch;
