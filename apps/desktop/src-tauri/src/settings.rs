//! The app's settings: one small JSON file, read before the window opens.
//!
//! A file that is missing, unreadable or not a settings file is the defaults: the app always
//! opens. A field this build does not know is kept out; a field the file lacks is its default.
//! Saving is crash-safe (a temp file, then a rename).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The smallest and largest keep-last-N the app offers.
pub const KEEP_RANGE: std::ops::RangeInclusive<u32> = 1..=9;

/// Everything the Settings page changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Keep last N slots by default. A board can set its own number.
    pub keep_last: u32,
    /// The one security switch: boards check signatures. Off by default (FRC setups expect it).
    pub security: bool,
    /// The release channel the app updates itself from.
    pub channel: String,
    /// `system`, `light` or `dark`.
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            keep_last: 3,
            security: false,
            channel: "dev".into(),
            theme: "system".into(),
        }
    }
}

impl Settings {
    /// The settings in `path`, or the defaults when it cannot be used.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .map(Self::checked)
            .unwrap_or_default()
    }

    /// Values outside what the app offers go back to their defaults.
    #[must_use]
    pub fn checked(mut self) -> Self {
        let defaults = Self::default();
        if !KEEP_RANGE.contains(&self.keep_last) {
            self.keep_last = defaults.keep_last;
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            self.theme = defaults.theme;
        }
        if !["dev", "beta", "stable"].contains(&self.channel.as_str()) {
            self.channel = defaults.channel;
        }
        self
    }

    /// Writes the settings to `path`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        std::fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_is_off_and_three_slots_are_kept_by_default() {
        let settings = Settings::default();
        assert!(!settings.security);
        assert_eq!(settings.keep_last, 3);
    }

    #[test]
    fn a_missing_or_broken_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn saved_settings_load_back_and_a_missing_field_is_its_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/settings.json");
        let settings = Settings {
            security: true,
            keep_last: 5,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        std::fs::write(&path, br#"{"security": true}"#).unwrap();
        let partial = Settings::load(&path);
        assert!(partial.security);
        assert_eq!(partial.keep_last, 3);
    }

    #[test]
    fn a_value_the_app_does_not_offer_goes_back_to_its_default() {
        let odd = Settings {
            keep_last: 0,
            theme: "sepia".into(),
            channel: "nightly".into(),
            security: true,
        }
        .checked();
        assert_eq!(odd.keep_last, 3);
        assert_eq!(odd.theme, "system");
        assert_eq!(odd.channel, "dev");
        assert!(odd.security);
    }
}
