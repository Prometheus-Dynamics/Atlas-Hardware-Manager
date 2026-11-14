//! The command-line rule a board states: each boot source must have its word on the cmdline.
//!
//! A board lists `boot_sources` (`<source>=<kernel path>` entries) and a `boot_args` prefix. A
//! slot's `boot/cmdline.txt` is the COMPLETE line the builder wrote (the installer appends
//! nothing), and it must carry one word per source: `<boot_args>.src.<source>=<kernel path>`.
//! A bundle whose cmdline lacks one is refused at install.
// LOCK: Lemnos L2 (I5i4, I5i5 final). Today the board file has no `boot_args`/`boot_sources`; the
// rule is the one Lemnos's boot_sources states. The default below is one source, the
// Pi's boot mode 1). At the lock move the board's own values replace it, read from its file.

use crate::BundleError;

/// The `boot_args` prefix and the `boot_sources` list of one board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootRule {
    /// The command-line prefix (`os` in the default).
    pub boot_args: String,
    /// `(source, kernel path)` for each medium the board can boot from.
    pub sources: Vec<(String, String)>,
}

impl Default for BootRule {
    /// The default rule: one source, `rpi-boot-mode-1` at its kernel path.
    fn default() -> Self {
        Self {
            boot_args: "os".into(),
            sources: vec![(
                "rpi-boot-mode-1".into(),
                "/soc@107c000000/mmc@fff000".into(),
            )],
        }
    }
}

impl BootRule {
    /// The word a cmdline must carry for one source: `<boot_args>.src.<source>=<kernel path>`.
    #[must_use]
    pub fn word(&self, source: &str, kernel_path: &str) -> String {
        format!("{}.src.{source}={kernel_path}", self.boot_args)
    }

    /// Checks a `boot/cmdline.txt` (its exact bytes) carries a word for every source.
    ///
    /// # Errors
    ///
    /// `MissingBootSource`, naming the first source whose word is absent.
    pub fn check_cmdline(&self, cmdline: &[u8]) -> Result<(), BundleError> {
        let text = String::from_utf8_lossy(cmdline);
        let words: Vec<&str> = text.split_ascii_whitespace().collect();
        for (source, kernel_path) in &self.sources {
            let word = self.word(source, kernel_path);
            if !words.iter().any(|candidate| *candidate == word) {
                return Err(BundleError::MissingBootSource {
                    boot_source: source.clone(),
                    word,
                });
            }
        }
        Ok(())
    }
}
