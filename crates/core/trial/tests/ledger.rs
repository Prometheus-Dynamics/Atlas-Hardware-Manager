//! The saved watches: what is kept, when it is written, the bound, and reporting once. The store is
//! in memory, with a switch that makes its writes fail, so no file is touched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use atlas_slots::SlotList;
use atlas_trial::{MAX_WATCHES, Read, Saved, Seen, TrialState, Trials, WatchStore};

/// The saved list, shared between the stores of one test, so a "restart" can reopen it.
#[derive(Debug, Clone, Default)]
struct Memory {
    saved: Arc<Mutex<Vec<Saved>>>,
    unreadable: Arc<AtomicBool>,
    unwritable: Arc<AtomicBool>,
}

impl WatchStore for Memory {
    fn load(&self) -> Result<Vec<Saved>, String> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err("the saved trial watches could not be read".into());
        }
        Ok(self.saved.lock().unwrap().clone())
    }

    fn save(&self, watches: &[Saved]) -> Result<(), String> {
        if self.unwritable.load(Ordering::SeqCst) {
            return Err("the state directory refused the write".into());
        }
        *self.saved.lock().unwrap() = watches.to_vec();
        Ok(())
    }
}

fn key(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn confirmed_list() -> SlotList {
    SlotList::parse(
        r#"{"slot_active":"slot-2","slot_default":"slot-2","slots":[],"trial":null,"resolved":{"slot":"slot-2","outcome":"confirmed","by":"auto","caller":null,"reason":"healthy","time":"T+40s"}}"#,
    )
    .expect("the list parses")
}

#[test]
fn a_started_watch_is_saved_and_followed_again_after_a_restart() {
    let store = Memory::default();
    let mut before = Trials::open(store.clone());
    let begun = before.begin(key(1), "slot-2", 100);
    assert!(begun.evicted.is_none());
    assert!(begun.saved.is_ok());
    drop(before);

    let mut after = Trials::open(store.clone());
    assert!(after.problem().is_none());
    let watch = after
        .watch(key(1))
        .expect("the watch is held after the restart");
    assert_eq!(watch.slot(), "slot-2");
    assert_eq!(watch.started_ms(), 100);
    assert!(matches!(watch.state(), TrialState::NotBackYet { .. }));

    // The board is seen again: its records say the trial held, and the watch is released once.
    let watch = after.watch_mut(key(1)).expect("held");
    assert_eq!(
        watch.seen(Seen::Found {
            key: key(1),
            running: Some("slot-2".into()),
        }),
        atlas_trial::Next::ReadRecords
    );
    watch.read(Read::List(confirmed_list()));
    let (reported, saved) = after.release(key(1));
    assert!(matches!(reported, Some(TrialState::Held(_))));
    assert!(saved.is_ok());
    assert!(after.watch(key(1)).is_none());
}

#[test]
fn an_outcome_is_reported_once_and_is_not_saved_again() {
    let store = Memory::default();
    let mut trials = Trials::open(store.clone());
    trials.begin(key(2), "slot-2", 1);
    let watch = trials.watch_mut(key(2)).expect("held");
    watch.seen(Seen::Away);
    watch.seen(Seen::Found {
        key: key(2),
        running: Some("slot-2".into()),
    });
    watch.read(Read::List(confirmed_list()));
    assert!(matches!(
        trials.release(key(2)).0,
        Some(TrialState::Held(_))
    ));
    assert!(
        trials.release(key(2)).0.is_none(),
        "a released watch is not reported again"
    );

    let reopened = Trials::open(store);
    assert!(
        reopened.watches().is_empty(),
        "nothing is left to report after the restart"
    );
}

#[test]
fn a_watch_that_is_still_waiting_is_not_released() {
    let store = Memory::default();
    let mut trials = Trials::open(store.clone());
    trials.begin(key(3), "slot-2", 1);
    trials.watch_mut(key(3)).expect("held").seen(Seen::Away);
    let (state, saved) = trials.release(key(3));
    assert!(state.is_none());
    assert!(saved.is_ok());
    assert_eq!(Trials::open(store).watches().len(), 1);
}

#[test]
fn a_new_trial_on_a_board_replaces_its_watch() {
    let store = Memory::default();
    let mut trials = Trials::open(store.clone());
    trials.begin(key(4), "slot-2", 1);
    trials.begin(key(4), "slot-3", 2);
    assert_eq!(trials.watches().len(), 1);
    assert_eq!(trials.watch(key(4)).expect("held").slot(), "slot-3");
    assert_eq!(Trials::open(store).watches().len(), 1);
}

#[test]
fn the_saved_list_is_bounded_and_the_oldest_watch_is_dropped_and_named() {
    let store = Memory::default();
    let mut trials = Trials::open(store.clone());
    for board in 0..u8::try_from(MAX_WATCHES).expect("the bound fits") {
        assert!(
            trials
                .begin(key(board), "slot-2", u64::from(board))
                .evicted
                .is_none()
        );
    }
    assert_eq!(trials.watches().len(), MAX_WATCHES);
    // One more: the oldest (started first, board 0) goes, and the drop is returned.
    let begun = trials.begin(key(200), "slot-2", 1_000);
    let evicted = begun.evicted.expect("the oldest watch is returned");
    assert_eq!(evicted.board(), key(0));
    assert_eq!(trials.watches().len(), MAX_WATCHES);
    assert_eq!(store.saved.lock().unwrap().len(), MAX_WATCHES);
    assert!(trials.watch(key(0)).is_none());
    assert!(trials.watch(key(200)).is_some());
}

#[test]
fn a_saved_list_over_the_bound_keeps_the_newest_on_open() {
    let store = Memory::default();
    {
        let mut trials = Trials::open(store.clone());
        for board in 0..20_u8 {
            trials.begin(key(board), "slot-2", u64::from(board));
        }
    }
    // Saving never goes over the bound, so the file is written over it here by hand.
    let mut over: Vec<Saved> = (0..20_u8)
        .map(|board| Saved {
            board: key(board),
            slot: "slot-2".into(),
            started_ms: u64::from(board),
        })
        .collect();
    over.reverse();
    *store.saved.lock().unwrap() = over;
    let trials = Trials::open(store);
    assert_eq!(trials.watches().len(), MAX_WATCHES);
    assert!(trials.watch(key(19)).is_some(), "the newest is kept");
    assert!(trials.watch(key(0)).is_none(), "the oldest is dropped");
}

#[test]
fn a_write_that_fails_keeps_the_watch_followed_and_says_it_is_not_saved() {
    let store = Memory::default();
    store.unwritable.store(true, Ordering::SeqCst);
    let mut trials = Trials::open(store.clone());
    let begun = trials.begin(key(5), "slot-2", 1);
    assert!(begun.saved.is_err(), "the failed write is reported");
    assert!(
        trials.watch(key(5)).is_some(),
        "the trial is still followed in memory"
    );
    assert!(store.saved.lock().unwrap().is_empty());
}

#[test]
fn a_saved_list_that_cannot_be_read_starts_empty_with_the_reason() {
    let store = Memory::default();
    store.unreadable.store(true, Ordering::SeqCst);
    let trials = Trials::open(store);
    assert!(trials.watches().is_empty());
    assert_eq!(
        trials.problem(),
        Some("the saved trial watches could not be read")
    );
}
