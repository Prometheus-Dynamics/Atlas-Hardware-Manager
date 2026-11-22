//! The real drivers and the computer's own links: what the app looks at outside the simulated
//! mode.
//!
//! The shell (F2) registers none: it shows the saved inventory and nothing new is found. Each
//! driver joins here with the item that puts its devices on a screen.

use crate::core::Parts;

/// The app's parts for the real mode.
#[must_use]
pub fn real() -> Parts {
    Parts::default()
}
