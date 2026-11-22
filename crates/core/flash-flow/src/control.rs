//! The person's hand on a running flow: stop the retries, try again now, cancel.

use std::sync::Arc;

use tokio::sync::{Notify, watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Run,
    Stopped,
    Cancelled,
}

/// Shared between the flow and whoever drives it (the page's buttons). Cheap to clone.
#[derive(Clone, Debug)]
pub struct Control {
    mode: Arc<watch::Sender<Mode>>,
    now: Arc<Notify>,
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Control {
    #[must_use]
    pub fn new() -> Self {
        Self {
            mode: Arc::new(watch::Sender::new(Mode::Run)),
            now: Arc::new(Notify::new()),
        }
    }

    /// Stops the retries (the manual off). A step that is running now finishes its try; after a failure it waits
    /// for [`Control::retry_now`] instead of retrying by itself.
    pub fn stop(&self) {
        self.mode.send_if_modified(|mode| {
            let change = *mode == Mode::Run;
            if change {
                *mode = Mode::Stopped;
            }
            change
        });
    }

    /// Tries again now: cuts a wait short, ends a stop, and asks a refused step once more. A press while no step
    /// waits does nothing: it is not kept for a later wait.
    pub fn retry_now(&self) {
        self.mode.send_if_modified(|mode| {
            let change = *mode == Mode::Stopped;
            if change {
                *mode = Mode::Run;
            }
            change
        });
        self.now.notify_waiters();
    }

    /// Cancels the flow. Final.
    pub fn cancel(&self) {
        self.mode.send_replace(Mode::Cancelled);
        self.now.notify_waiters();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.mode.borrow() == Mode::Cancelled
    }

    #[must_use]
    pub fn is_stopped(&self) -> bool {
        *self.mode.borrow() == Mode::Stopped
    }

    /// Resolves once the flow is cancelled.
    pub(crate) async fn cancelled(&self) {
        let mut mode = self.mode.subscribe();
        // The sender lives in `self`, so the channel never closes while this waits.
        let _ = mode.wait_for(|mode| *mode == Mode::Cancelled).await;
    }

    /// Resolves when the person asks to try again now (or cancels: the caller checks).
    pub(crate) async fn asked_now(&self) {
        self.now.notified().await;
    }

    /// Resolves once the person stops the retries.
    pub(crate) async fn stopped(&self) {
        let mut mode = self.mode.subscribe();
        let _ = mode.wait_for(|mode| *mode == Mode::Stopped).await;
    }

    /// Resolves once the retries are not stopped (or the flow is cancelled).
    pub(crate) async fn running(&self) {
        let mut mode = self.mode.subscribe();
        let _ = mode.wait_for(|mode| *mode != Mode::Stopped).await;
    }
}
