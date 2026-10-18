// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    // Invoked as `atlas` or with a CLI command: run the CLI before anything
    // touches GTK or WebKit, so it works without a display. Not on Windows,
    // where this is a GUI-subsystem program without a console; the CLI is
    // its own atlas.exe there.
    #[cfg(not(windows))]
    if let Some(args) = atlas_cli::cli_args(&std::env::args_os().collect::<Vec<_>>()) {
        return atlas_cli::run(args);
    }
    atlas_app_lib::run();
    ExitCode::SUCCESS
}
