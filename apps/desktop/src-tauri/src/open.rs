//! Opening something outside the app: a folder in the file manager, a page in the browser.
//!
//! The window sets the system's opener. A build without a window (the UI bridge) keeps what was
//! asked for, so a test can read it; nothing is opened there.

use std::sync::Mutex;

/// Opens a path or an address with the system's own program for it.
pub trait Opener: Send + Sync {
    fn open(&self, target: &str) -> Result<(), String>;
}

/// The opener of a build with no window: it opens nothing and remembers what was asked.
#[derive(Debug, Default)]
pub struct Remembered {
    asked: Mutex<Vec<String>>,
}

impl Remembered {
    #[must_use]
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Opener for Remembered {
    fn open(&self, target: &str) -> Result<(), String> {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(target.to_owned());
        Ok(())
    }
}
