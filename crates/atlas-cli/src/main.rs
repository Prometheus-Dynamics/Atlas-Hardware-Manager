//! `atlas.exe` on Windows, and `cargo run -p atlas-cli` in a checkout. The
//! Linux and macOS installers run the same CLI from the desktop app.

use std::process::ExitCode;

fn main() -> ExitCode {
    atlas_cli::run(std::env::args_os())
}
