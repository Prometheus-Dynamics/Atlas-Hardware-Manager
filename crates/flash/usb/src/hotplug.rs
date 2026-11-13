//! Hotplug: a watch calls back for every device that attaches or detaches,
//! so a loader can look right away instead of polling.

use std::future::poll_fn;

use futures_core::Stream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::gauge::{Gauges, Kind, Tracked};
use crate::nusb_backend::usb_events;
use crate::{DeviceSummary, UsbError};

/// A device arrived on the bus, or left it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotplugEvent {
    Attached(DeviceSummary),
    /// The device at `location` is gone. Only the location is known for a
    /// device that left, because the bus no longer describes it.
    Detached {
        location: String,
    },
}

/// A running watch. Dropping it stops the watch; [`Watch::close`] stops it
/// and waits until its task has ended.
#[derive(Debug)]
pub struct Watch {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    _watcher: Tracked,
}

impl Watch {
    /// Stops the watch and waits until its task has ended.
    pub async fn close(mut self) {
        drop(self.stop.take());
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

/// Calls `on_event` for each event of `events`, on a tokio task, until the
/// returned [`Watch`] is dropped or closed or the stream ends. The task
/// holds `gauges` counts while it runs, so a leaked task shows up in
/// [`Gauges::live`].
pub fn spawn_watch<S, F>(gauges: &Gauges, events: S, mut on_event: F) -> Watch
where
    S: Stream<Item = HotplugEvent> + Send + 'static,
    F: FnMut(HotplugEvent) + Send + 'static,
{
    let (stop, mut stopped) = oneshot::channel::<()>();
    let watcher = Tracked::new(gauges, Kind::Watcher);
    let task_count = Tracked::new(gauges, Kind::Task);
    let task = tokio::spawn(async move {
        let _count = task_count;
        let mut events = Box::pin(events);
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped => break,
                next = poll_fn(|cx| events.as_mut().poll_next(cx)) => match next {
                    Some(event) => on_event(event),
                    None => break,
                },
            }
        }
    });
    Watch {
        stop: Some(stop),
        task: Some(task),
        _watcher: watcher,
    }
}

/// Watches the real bus, counted in [`Gauges::global`]. Needs a tokio
/// runtime.
pub fn watch_usb(on_event: impl FnMut(HotplugEvent) + Send + 'static) -> Result<Watch, UsbError> {
    let events = usb_events()?;
    Ok(spawn_watch(&Gauges::global(), events, on_event))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::sim::{Answer, SimBus};
    use crate::{Live, OpenOptions, UsbBus};

    fn summary(location: &str) -> DeviceSummary {
        DeviceSummary {
            location: location.into(),
            vendor: 0x0a5c,
            product: 0x2764,
            serial: None,
        }
    }

    /// Lets spawned tasks run until `done` holds. Fails if they never settle.
    async fn settle(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..10_000 {
            if done() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("{what} did not settle");
    }

    #[tokio::test]
    async fn watch_delivers_events_in_order_and_closes_cleanly() {
        let bus = SimBus::new();
        let gauges = bus.gauges();
        let seen = Arc::new(Mutex::new(Vec::<HotplugEvent>::new()));
        let kept = Arc::clone(&seen);
        let watch = spawn_watch(&gauges, bus.subscribe(), move |event| {
            kept.lock().unwrap_or_else(|e| e.into_inner()).push(event);
        });

        bus.attach(summary("1-2"));
        bus.detach("1-2");
        settle("two events", || {
            seen.lock().unwrap_or_else(|e| e.into_inner()).len() == 2
        })
        .await;
        assert_eq!(
            gauges.live(),
            Live {
                watchers: 1,
                tasks: 1,
                devices: 0
            }
        );

        watch.close().await;
        assert_eq!(gauges.live(), Live::default());
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            *seen,
            vec![
                HotplugEvent::Attached(summary("1-2")),
                HotplugEvent::Detached {
                    location: "1-2".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn dropping_a_watch_ends_its_task() {
        let bus = SimBus::new();
        let gauges = bus.gauges();
        let watch = spawn_watch(&gauges, bus.subscribe(), |_| {});
        assert_eq!(gauges.live().tasks, 1);
        drop(watch);
        settle("the dropped watch's task", || {
            gauges.live() == Live::default()
        })
        .await;
    }

    /// Attach, transfer, reset, re-enumerate, unplug: many times over, with
    /// watches coming and going. Every watch, task, device handle and the
    /// bus's own memory must be gone afterwards.
    #[tokio::test(start_paused = true)]
    async fn hotplug_storm_leaks_no_handles_tasks_or_memory() {
        const CYCLES: usize = 300;
        let bus = SimBus::new();
        let gauges = bus.gauges();
        let probe = bus.probe();
        let counted = Arc::new(AtomicUsize::new(0));

        let long_counter = Arc::clone(&counted);
        let long = spawn_watch(&gauges, bus.subscribe(), move |_| {
            long_counter.fetch_add(1, Ordering::SeqCst);
        });

        for cycle in 0..CYCLES {
            // Short-lived watches, half closed and half just dropped.
            for _ in 0..3 {
                let watch = spawn_watch(&gauges, bus.subscribe(), |_| {});
                if cycle % 2 == 0 {
                    watch.close().await;
                } else {
                    drop(watch);
                }
            }

            let location = format!("1-{}", cycle % 4 + 1);
            bus.attach(summary(&location));
            let options = OpenOptions::default();
            if cycle % 2 == 0 {
                // The device resets mid-transfer and never comes back here.
                let mut io = bus.open(&location, &options).await.unwrap();
                bus.script(&location, [Answer::Disconnect]);
                assert_eq!(
                    io.bulk_out(&[0; 4]).await,
                    Err(crate::UsbError::Disconnected)
                );
            } else {
                let io = bus.open(&location, &options).await.unwrap();
                drop(io);
                assert!(bus.detach(&location));
            }
            tokio::task::yield_now().await;
        }

        // Two events per cycle, each seen by the long-lived watch.
        settle("every hotplug event", || {
            counted.load(Ordering::SeqCst) == 2 * CYCLES
        })
        .await;
        settle("the short-lived watches", || {
            gauges.live()
                == Live {
                    watchers: 1,
                    tasks: 1,
                    devices: 0,
                }
        })
        .await;

        long.close().await;
        drop(bus);
        settle("everything", || gauges.live() == Live::default()).await;
        assert_eq!(gauges.live(), Live::default());
        assert!(probe.upgrade().is_none(), "the bus's state was not freed");
    }

    #[test]
    fn real_watch_and_open_need_nusb_tokio_feature() {
        // Opening a device blocks without nusb's tokio feature; that once hung
        // every flash. Keep it on.
        let manifest = include_str!("../Cargo.toml");
        let line = manifest
            .lines()
            .find(|line| line.trim_start().starts_with("nusb"))
            .expect("nusb is a dependency");
        assert!(line.contains("\"tokio\""), "nusb must enable tokio: {line}");
    }
}
