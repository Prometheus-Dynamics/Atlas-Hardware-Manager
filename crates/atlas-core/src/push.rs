//! Devices' push channels (a board's board-stream). While Atlas watches,
//! every live device whose status capability has one is followed: what it
//! pushes (a new event, an update-state change, a reconnect) makes Atlas
//! fetch the device's new events and tell hosts to re-read its status at
//! once, instead of on the next poll; an update-state change also re-scans,
//! so the device's identity (its update state, slot) is current.
//!
//! A channel that drops (a reboot, a cable) re-scans at once and then every
//! [`LOOK_EVERY`] for [`LOOK_FOR`], so a board that restarts is seen going
//! and coming back within seconds; after that the follower only reconnects,
//! every [`RETRY_EVERY`], while the device stays live.

use std::sync::Arc;
use std::time::{Duration, Instant};

use atlas_driver::{DeviceKey, PushSink, StatusPush};
use tokio::sync::mpsc;

use crate::{Atlas, Event};

pub(crate) const LOOK_EVERY: Duration = Duration::from_secs(2);
pub(crate) const LOOK_FOR: Duration = Duration::from_secs(120);
pub(crate) const RETRY_EVERY: Duration = Duration::from_secs(10);

impl Atlas {
    /// After a scan, while watching: follows each live device's push
    /// channel that isn't followed yet.
    pub(crate) fn follow_push_channels(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let new: Vec<DeviceKey> = {
            let mut state = self.inner.state();
            let keys: Vec<DeviceKey> = state
                .live
                .iter()
                .filter(|(_, live)| live.capabilities.status.is_some())
                .map(|(key, _)| key.clone())
                .filter(|key| !state.followed.contains(key))
                .collect();
            state.followed.extend(keys.iter().cloned());
            keys
        };
        for key in new {
            let atlas = self.clone();
            runtime.spawn(async move { atlas.follow(key).await });
        }
    }

    /// Follows one device's push channel until it has none or is gone.
    async fn follow(self, key: DeviceKey) {
        // Since the channel dropped: while recent, look for the device often.
        let mut lost: Option<Instant> = None;
        let mut last_state: Option<String> = None;
        loop {
            let found = self
                .live_device(&key)
                .ok()
                .and_then(|(live, record)| live.capabilities.status.map(|status| (status, record)));
            let Some((status, record)) = found else {
                if lost.is_some_and(|since| since.elapsed() < LOOK_FOR) {
                    self.inner.changes.notify_one();
                    tokio::time::sleep(LOOK_EVERY).await;
                    continue;
                }
                break;
            };
            let (tx, mut rx) = mpsc::unbounded_channel();
            let sink: PushSink = Arc::new(move |push| {
                let _ = tx.send(push);
            });
            let identity = record.identity.clone();
            let mut watching = tokio::spawn(async move { status.watch(&identity, sink).await });
            let mut connected = false;
            let ended = loop {
                tokio::select! {
                    Some(push) = rx.recv() => {
                        let mut batch = vec![push];
                        while let Ok(more) = rx.try_recv() {
                            batch.push(more);
                        }
                        if batch.iter().any(|push| matches!(push, StatusPush::Hello { .. })) {
                            lost = None;
                            connected = true;
                        }
                        self.on_pushes(&key, &batch, &mut last_state).await;
                    }
                    result = &mut watching => break result.unwrap_or(Ok(true)),
                }
            };
            match ended {
                // No push channel: the device is polled as before.
                Ok(false) => break,
                Ok(true) | Err(_) => {
                    // A channel that was open dropped: the device went, or
                    // is restarting. One that never opened only retries.
                    if connected {
                        lost = Some(Instant::now());
                    }
                    if lost.is_some_and(|since| since.elapsed() < LOOK_FOR) {
                        // Look now, not at the next scan.
                        self.inner.changes.notify_one();
                        tokio::time::sleep(LOOK_EVERY).await;
                    } else {
                        tokio::time::sleep(RETRY_EVERY).await;
                    }
                }
            }
        }
        self.inner.state().followed.remove(&key);
    }

    async fn on_pushes(
        &self,
        key: &DeviceKey,
        batch: &[StatusPush],
        last_state: &mut Option<String>,
    ) {
        let mut rescan = false;
        let mut sync = false;
        for push in batch {
            match push {
                StatusPush::Hello { .. } => sync = true,
                StatusPush::Event(_) => sync = true,
                StatusPush::Update { state } => {
                    if last_state.as_deref() != Some(state.as_str()) {
                        rescan = true;
                        *last_state = Some(state.clone());
                    }
                }
            }
        }
        if rescan {
            self.inner.changes.notify_one();
        }
        if sync {
            let _ = self.sync_device_events(key).await;
        }
        self.inner
            .events
            .emit(Event::DeviceStatus { key: key.clone() });
    }
}
