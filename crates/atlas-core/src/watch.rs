//! Live discovery: instead of polling, Atlas scans when a link source says
//! something changed (USB hotplug, an mDNS announcement), plus a slow
//! fallback rescan for sources that cannot watch and for devices that leave
//! without saying so.

use std::time::Duration;

use atlas_driver::ChangeNotifier;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::{Atlas, CoreError};

#[derive(Clone, Debug)]
pub struct WatchOptions {
    /// How long to wait after a change notice for more to arrive, so a
    /// device that shows up as several USB interfaces costs one scan.
    pub debounce: Duration,
    /// Rescan this often even without notices.
    pub fallback: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(300),
            fallback: Duration::from_secs(20),
        }
    }
}

/// How Atlas is keeping the inventory current, for status displays.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DiscoveryStatus {
    /// True while a watch is running.
    pub live: bool,
    /// Sources that push changes, for example `USB` and `Network`.
    pub watching: Vec<String>,
    /// Sources Atlas can only poll, on the fallback timer.
    pub polled: Vec<String>,
    pub fallback_ms: u64,
}

impl Atlas {
    /// Scans now, then whenever a link source reports a change, and on the
    /// fallback timer. Runs until the returned token is cancelled; watching
    /// again later reuses the same source subscriptions. Must be called from
    /// inside a Tokio runtime.
    pub fn watch(&self, options: WatchOptions) -> Result<CancellationToken, CoreError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| CoreError::NoRuntime)?;
        let token = CancellationToken::new();
        let notify = self.inner.changes.clone();

        // Sources are subscribed once per process; their threads live on.
        let (watching, polled) = self
            .inner
            .subscriptions
            .get_or_init(|| {
                let mut watching = Vec::new();
                let mut polled = Vec::new();
                for source in &self.inner.link_sources {
                    let wake = notify.clone();
                    let notifier = ChangeNotifier::new(move || wake.notify_one());
                    let name = source.name().to_string();
                    let list = if source.watch(notifier) {
                        &mut watching
                    } else {
                        &mut polled
                    };
                    if !list.contains(&name) {
                        list.push(name);
                    }
                }
                (watching, polled)
            })
            .clone();
        *self.inner.discovery() = DiscoveryStatus {
            live: true,
            watching,
            polled,
            fallback_ms: u64::try_from(options.fallback.as_millis()).unwrap_or(u64::MAX),
        };

        let atlas = self.clone();
        let stop = token.clone();
        runtime.spawn(async move {
            loop {
                atlas.scan().await;
                atlas.follow_push_channels();
                tokio::select! {
                    () = stop.cancelled() => break,
                    () = notify.notified() => {
                        // Let a burst of notices settle into one scan.
                        tokio::select! {
                            () = stop.cancelled() => break,
                            () = tokio::time::sleep(options.debounce) => {}
                        }
                    }
                    () = tokio::time::sleep(options.fallback) => {}
                }
            }
            atlas.inner.discovery().live = false;
        });
        Ok(token)
    }

    pub fn discovery(&self) -> DiscoveryStatus {
        self.inner.discovery().clone()
    }
}
