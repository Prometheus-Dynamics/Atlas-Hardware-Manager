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
    // Only the app: the CLI above has no web view.
    #[cfg(target_os = "linux")]
    cap_malloc_arenas();
    atlas_app_lib::run();
    ExitCode::SUCCESS
}

/// WebKitGTK paints on a pool of threads (Skia's CPU workers, the
/// compositor, image decoders), and glibc gives each its own malloc arena,
/// which it rarely hands back: over a day the web view's process held ~1 GB
/// of them, mostly free and swapped out. Two arenas keep that bounded. The
/// web view's processes inherit the setting from this one, which takes it on
/// starting again in place (same pid, same arguments); a value already set
/// is kept, and if that fails Atlas starts as it is.
#[cfg(target_os = "linux")]
fn cap_malloc_arenas() {
    use std::os::unix::process::CommandExt;
    if std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut command = std::process::Command::new(exe);
    if let Some(name) = std::env::args_os().next() {
        command.arg0(name);
    }
    let error = command
        .args(std::env::args_os().skip(1))
        .env("MALLOC_ARENA_MAX", "2")
        .exec();
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("atlas: starting with fewer malloc arenas failed ({error}); going on without\n")
            .as_bytes(),
    );
}
