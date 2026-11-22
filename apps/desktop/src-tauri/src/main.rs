//! The `atlas-app` binary: opens the window.

// No console window on Windows in a release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    atlas_app::started();
    atlas_app::window::run();
}
