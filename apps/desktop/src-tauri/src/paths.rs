//! Where the app keeps its files.
//!
//! - data (`$XDG_DATA_HOME/atlas`, else `~/.local/share/atlas`): the inventory, the job list and
//!   the settings.
//! - state (`$XDG_STATE_HOME/atlas`, else `~/.local/state/atlas`): the app's own log.
//!
//! `ATLAS_DATA_DIR` moves both under one directory (`<dir>/data`, `<dir>/state`): the tests and
//! the simulated mode use it, so they never read or write a person's real inventory.

use std::ffi::OsString;
use std::path::PathBuf;

/// The app's directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dirs {
    pub data: PathBuf,
    pub state: PathBuf,
}

impl Dirs {
    /// Both directories under one root.
    #[must_use]
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            data: root.join("data"),
            state: root.join("state"),
        }
    }

    /// The directories for this process, from its environment. `None` when no variable names a
    /// place (no home directory): the app then runs without saving.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let set = |name: &str| {
            var(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        if let Some(root) = set("ATLAS_DATA_DIR") {
            return Some(Self::under(root));
        }
        let home = set("HOME").or_else(|| set("USERPROFILE"));
        let data = set("XDG_DATA_HOME")
            .or_else(|| set("APPDATA"))
            .or_else(|| home.as_ref().map(|home| home.join(".local/share")))?;
        let state = set("XDG_STATE_HOME")
            .or_else(|| set("LOCALAPPDATA"))
            .or_else(|| home.as_ref().map(|home| home.join(".local/state")))?;
        Some(Self {
            data: data.join("atlas"),
            state: state.join("atlas"),
        })
    }

    #[must_use]
    pub fn inventory(&self) -> PathBuf {
        self.data.join("inventory.json")
    }

    #[must_use]
    pub fn jobs(&self) -> PathBuf {
        self.data.join("jobs.json")
    }

    #[must_use]
    pub fn settings(&self) -> PathBuf {
        self.data.join("settings.json")
    }

    #[must_use]
    pub fn logs(&self) -> PathBuf {
        self.state.join("log")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_override_moves_both_directories_under_one_root() {
        let dirs = Dirs::from_vars(vars(&[("ATLAS_DATA_DIR", "/x"), ("HOME", "/home/a")])).unwrap();
        assert_eq!(dirs, Dirs::under("/x"));
        assert_eq!(dirs.inventory(), PathBuf::from("/x/data/inventory.json"));
        assert_eq!(dirs.logs(), PathBuf::from("/x/state/log"));
    }

    #[test]
    fn xdg_directories_are_used_when_set_and_home_otherwise() {
        let dirs = Dirs::from_vars(vars(&[("XDG_DATA_HOME", "/d"), ("HOME", "/home/a")])).unwrap();
        assert_eq!(dirs.data, PathBuf::from("/d/atlas"));
        assert_eq!(dirs.state, PathBuf::from("/home/a/.local/state/atlas"));
    }

    #[test]
    fn no_home_and_no_variable_is_no_directories() {
        assert_eq!(Dirs::from_vars(vars(&[("HOME", "")])), None);
    }
}
