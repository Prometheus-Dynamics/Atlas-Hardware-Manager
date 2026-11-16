//! Scratch directories for the session tests, one place for every file that starts a node.
//!
//! A node's directory is made by `atlas_types::socket::node_dir`: short whatever the name, under
//! `$XDG_RUNTIME_DIR` (else the temp dir), so its socket path always fits a Unix socket.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use atlas_types::socket::{NODE_SOCKET, node_dir};

/// A scratch directory for one test, for a node to start in. Any name works: the directory's name
/// is short and the readable name is kept in its `name` file.
///
/// # Panics
///
/// When the directory cannot be made (the error names the path).
#[must_use]
pub fn scratch(name: &str) -> atlas_sim_node::socket::NodeDir {
    node_dir(name).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The path of a node's socket in `dir`, as Orion's stub makes it.
#[must_use]
pub fn socket_in(dir: &Path) -> PathBuf {
    dir.join(NODE_SOCKET)
}
