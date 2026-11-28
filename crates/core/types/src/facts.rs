//! The board's identity facts, as the Lemnos pin publishes them. One list: every Atlas reader of a
//! Lemnos fact takes its names and its limits from here.

// board: Lemnos 98fbe0c2 engine/identity/src/facts.rs:19-28 (NAMES) and service/orion/src/hello.rs:23 (OFFERS)
/// The facts a node publishes only when Lemnos runs on it: the identity device's names, and `offers`,
/// which the bridge computes. A node with any of them is a Lemnos node.
pub const LEMNOS_FACTS: [&str; 8] = [
    "family",
    "serial",
    "model",
    "version.slot",
    "version.lemnos",
    "version.product",
    "version.os",
    "offers",
];

// board: Lemnos 98fbe0c2 service/orion/src/hello.rs:19 (MAX_VALUE_BYTES)
/// The most bytes one hello fact's value may carry at the pin. Orion's hello document holds at least
/// this much per fact, and a longer `offers` is refused by the bridge's own limit, not cut short.
pub const FACT_MAX_BYTES: usize = 1024;
