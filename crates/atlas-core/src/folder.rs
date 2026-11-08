//! Where Atlas keeps its files. The app and the command line resolve the
//! folder the same way, so they share settings, inventory and the Orion
//! operator key.

use std::ffi::OsString;
use std::path::PathBuf;

/// Atlas's data folder: `ATLAS_DATA_DIR` when set, else `atlas` under the
/// user's data directory (`$XDG_DATA_HOME`, else `~/.local/share`), else
/// `atlas` in the temporary directory.
pub fn data_dir() -> PathBuf {
    data_dir_from(std::env::var_os("ATLAS_DATA_DIR"), dirs::data_dir())
}

/// [`data_dir`] with its inputs given: the override, and the user's data
/// directory.
pub fn data_dir_from(override_dir: Option<OsString>, user_data: Option<PathBuf>) -> PathBuf {
    override_dir
        .map(PathBuf::from)
        .or_else(|| user_data.map(|dir| dir.join("atlas")))
        .unwrap_or_else(|| std::env::temp_dir().join("atlas"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::data_dir_from;

    #[test]
    fn the_override_is_the_data_folder_itself() {
        let dir = data_dir_from(
            Some("/scratch/atlas-test".into()),
            Some("/home/u/.local/share".into()),
        );
        assert_eq!(dir, PathBuf::from("/scratch/atlas-test"));
    }

    #[test]
    fn without_an_override_the_user_data_folder_holds_atlas() {
        let dir = data_dir_from(None, Some("/home/u/.local/share".into()));
        assert_eq!(dir, Path::new("/home/u/.local/share/atlas"));
    }

    #[test]
    fn without_either_atlas_falls_back_to_the_temporary_folder() {
        assert_eq!(
            data_dir_from(None, None),
            std::env::temp_dir().join("atlas")
        );
    }
}
