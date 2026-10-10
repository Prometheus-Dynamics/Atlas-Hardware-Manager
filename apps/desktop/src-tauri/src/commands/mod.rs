//! Tauri commands. Each maps to one core intent; errors are user-facing
//! sentences. The TypeScript side of this contract lives in
//! `apps/desktop/src/lib/api`.

pub mod devices;
pub mod jobs;
pub mod nt;
pub mod orion;
pub mod releases;
pub mod robots;
pub mod system;

pub type CmdResult<T> = Result<T, String>;

pub(crate) fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}
