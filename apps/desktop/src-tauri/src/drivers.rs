//! Hardware drivers compiled into the app.

use atlas_core::AtlasBuilder;

use crate::settings::AppPaths;

/// Registers every hardware driver and link source this build includes.
pub fn register_hardware(
    builder: AtlasBuilder,
    _paths: &AppPaths,
    _warnings: &mut Vec<String>,
) -> AtlasBuilder {
    builder
}
