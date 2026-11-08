//! Serving this computer's clock to boards (Orion's `operator` time source,
//! `docs/time-sync.md`).
//!
//! Every operator connection the app holds to a node or a board offers the
//! computer's clock through Orion's `serve_time`, while the Settings toggle
//! is on and the node granted the operator `clock.set`. Orion's loop waits at
//! the node's pace, and retries quietly after a refusal or an older node: that
//! is not an error to show. A [`ClockServer`] lives exactly as long as its
//! connection: dropping it aborts the offers.

use std::sync::LazyLock;

use orion_client::remote::RemoteOperator;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// A computer kept by NTP (the stratum the node's sync weighs).
pub const STRATUM: u8 = 3;
/// The action grant serving the clock needs: the same power as setting it.
pub const GRANT: &str = "clock.set";

/// The Settings toggle, shared by every connection. On by default.
static SERVING: LazyLock<watch::Sender<bool>> = LazyLock::new(|| watch::channel(true).0);

/// Turns serving the clock on or off for every connection, now.
pub fn set_serve_clock(on: bool) {
    SERVING.send_replace(on);
}

/// Whether this computer serves its clock to boards.
pub fn serve_clock() -> bool {
    *SERVING.borrow()
}

/// Offers this computer's clock on one operator connection until dropped.
pub struct ClockServer {
    task: JoinHandle<()>,
}

impl ClockServer {
    /// Starts serving on `operator`, when the operator holds the grant. The
    /// setting is followed while it runs: off stops the offers, on starts
    /// them again. `None` when the operator lacks the grant (the node
    /// refuses it, and a grant made later applies at the next connection).
    ///
    /// Call from inside the Tokio runtime.
    pub fn start(operator: &RemoteOperator) -> Option<Self> {
        if !operator.welcome().allows_action(GRANT) {
            return None;
        }
        let operator = operator.clone();
        let mut setting = SERVING.subscribe();
        let task = tokio::spawn(async move {
            // The offers run while the setting is on; the inner handle is
            // aborted with this task (and when the setting turns off).
            let mut offers: Option<Offers> = None;
            loop {
                let on = *setting.borrow_and_update();
                if on && offers.is_none() {
                    offers = Some(Offers(operator.serve_time(STRATUM)));
                } else if !on {
                    offers = None;
                }
                if setting.changed().await.is_err() {
                    break;
                }
            }
        });
        Some(Self { task })
    }
}

impl Drop for ClockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The offer loop's task, aborted when dropped.
struct Offers(JoinHandle<()>);

impl Drop for Offers {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serving_is_on_by_default_and_toggles() {
        assert!(serve_clock());
        set_serve_clock(false);
        assert!(!serve_clock());
        set_serve_clock(true);
        assert!(serve_clock());
    }
}
