//! Writing the inventory off the caller's thread, coalesced: a burst of
//! changes (a scan, a board's event sync, an action) is one write a moment
//! later, and a snapshot equal to the last one written isn't written again.
//! With no delay ([`crate::AtlasOptions::persist_delay`] zero) every save is
//! written at once, on the caller's thread, as tests and the CLI expect.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::events::EventBus;
use crate::{Event, InventoryStore, Snapshot};

enum Message {
    Save(Box<Snapshot>),
    Flush(SyncSender<()>),
}

pub(crate) struct Writer {
    store: Arc<dyn InventoryStore>,
    events: EventBus,
    /// Set when saves are delayed: the writer thread's queue.
    queue: Option<Mutex<Sender<Message>>>,
    /// What was written last (immediate mode), so an unchanged one is skipped.
    last: Mutex<Option<Snapshot>>,
}

impl Writer {
    pub(crate) fn new(store: Arc<dyn InventoryStore>, events: EventBus, delay: Duration) -> Self {
        let queue = (!delay.is_zero()).then(|| {
            let (tx, rx) = mpsc::channel();
            let (store, events) = (store.clone(), events.clone());
            std::thread::Builder::new()
                .name("atlas-inventory".into())
                .spawn(move || run(rx, store.as_ref(), &events, delay))
                .map(|_| Mutex::new(tx))
        });
        Self {
            store,
            events,
            queue: queue.and_then(Result::ok),
            last: Mutex::new(None),
        }
    }

    pub(crate) fn save(&self, snapshot: Snapshot) {
        if let Some(queue) = &self.queue {
            let sent = queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .send(Message::Save(Box::new(snapshot.clone())));
            if sent.is_ok() {
                return;
            }
        }
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.as_ref() == Some(&snapshot) {
            return;
        }
        if write(self.store.as_ref(), &self.events, &snapshot) {
            *last = Some(snapshot);
        }
    }

    /// Waits until every save asked for so far is on disk.
    pub(crate) fn flush(&self) {
        let Some(queue) = &self.queue else { return };
        let (done, wait) = mpsc::sync_channel(1);
        let sent = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send(Message::Flush(done));
        if sent.is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(10));
        }
    }
}

fn write(store: &dyn InventoryStore, events: &EventBus, snapshot: &Snapshot) -> bool {
    match store.save(snapshot) {
        Ok(()) => true,
        Err(error) => {
            events.emit(Event::ScanWarning {
                message: format!("Could not save the device inventory: {error}"),
            });
            false
        }
    }
}

fn run(rx: Receiver<Message>, store: &dyn InventoryStore, events: &EventBus, delay: Duration) {
    let mut written: Option<Snapshot> = None;
    let mut pending: Option<Box<Snapshot>> = None;
    let mut due = Instant::now();
    loop {
        let message = match &pending {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(_) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
        };
        let mut flushed = None;
        match message {
            Ok(Message::Save(snapshot)) => {
                if pending.is_none() {
                    due = Instant::now() + delay;
                }
                pending = Some(snapshot);
                continue;
            }
            Ok(Message::Flush(done)) => flushed = Some(done),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if pending.is_none() {
                    return;
                }
            }
        }
        if let Some(snapshot) = pending.take()
            && written.as_ref() != Some(&*snapshot)
            && write(store, events, &snapshot)
        {
            written = Some(*snapshot);
        }
        if let Some(done) = flushed {
            let _ = done.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counting {
        inner: MemoryStore,
        writes: AtomicUsize,
    }

    impl InventoryStore for Counting {
        fn load(&self) -> Result<Snapshot, crate::StoreError> {
            self.inner.load()
        }
        fn save(&self, snapshot: &Snapshot) -> Result<(), crate::StoreError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.inner.save(snapshot)
        }
    }

    fn snapshot(n: usize) -> Snapshot {
        Snapshot {
            board_events: (0..n)
                .map(|i| crate::BoardEventLog {
                    board_serial: format!("board {i}"),
                    events: Vec::new(),
                })
                .collect(),
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_burst_of_saves_is_one_write_of_the_last() {
        let store = Arc::new(Counting::default());
        let writer = Writer::new(store.clone(), EventBus::new(8), Duration::from_millis(50));
        for n in 1..=20 {
            writer.save(snapshot(n));
        }
        writer.flush();
        assert_eq!(store.writes.load(Ordering::SeqCst), 1);
        assert_eq!(store.load().unwrap(), snapshot(20));
        writer.save(snapshot(20));
        writer.flush();
        assert_eq!(
            store.writes.load(Ordering::SeqCst),
            1,
            "unchanged: not written again"
        );
    }

    #[test]
    fn with_no_delay_each_change_is_written_at_once() {
        let store = Arc::new(Counting::default());
        let writer = Writer::new(store.clone(), EventBus::new(8), Duration::ZERO);
        writer.save(snapshot(1));
        writer.save(snapshot(1));
        writer.save(snapshot(2));
        assert_eq!(store.writes.load(Ordering::SeqCst), 2);
        assert_eq!(store.load().unwrap(), snapshot(2));
    }
}
