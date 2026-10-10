//! A device's state and history: what Atlas did to it (its fleet activity
//! entries) merged with the board's own event log, which also has what
//! Orion and people on the board did.
//!
//! Board events are kept per physical board (its board serial, like
//! self-tests), at most [`BOARD_EVENTS_LIMIT`], and saved with the
//! inventory. They are fetched when a device's status is read and, at most
//! every [`SYNC_EVERY`], after a scan: by seq from a board that numbers its
//! events (whatever its clock did), else by time. An event already kept
//! (same boot and seq, or without a seq the same boot, time, kind and
//! message) is not added again. New events that someone other
//! than Atlas caused, and that matter (an update's outcome, an unclean boot,
//! new SSH keys), also get a line in the fleet history.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use atlas_driver::{DeviceEvent, DeviceKey, DeviceStatus, EventPage, EventQuery, EventSource};
use serde::{Deserialize, Serialize};

use crate::activity::{ActivityEntry, ActivityKind, ActivityLevel};
use crate::selftest::board_of;
use crate::time::now_ms;
use crate::{Atlas, CoreError, DeviceRecord, Event};

/// Board events kept per board.
pub const BOARD_EVENTS_LIMIT: usize = 500;
/// Events asked for at a time, and the pages one sync may fetch.
const PAGE: usize = 200;
const MAX_PAGES: usize = 5;
/// A boot's clock this close to this computer's counts as right.
const CLOCK_SLACK_S: i64 = 2;
/// How often a scan fetches a board's new events.
pub(crate) const SYNC_EVERY: Duration = Duration::from_secs(60);

/// One board's kept events, oldest first (for the saved inventory).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardEventLog {
    pub board_serial: String,
    pub events: Vec<DeviceEvent>,
}

/// Where a history entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryOrigin {
    /// Atlas's own record (fleet activity).
    Atlas,
    /// The board's event log.
    Board,
}

/// One line of a device's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Atlas's clock for its own entries; for board events, the board's
    /// clock corrected by that boot's offset when Atlas knew it.
    pub at_ms: u64,
    pub level: ActivityLevel,
    /// Atlas's activity kind (`update-result`, …) or the board event's kind
    /// (`update.staged`, `boot`, …).
    pub kind: String,
    /// Who did it: Atlas for its own entries, the event's source otherwise.
    pub source: EventSource,
    pub origin: HistoryOrigin,
    pub message: String,
    #[serde(default)]
    pub boot_id: Option<String>,
    #[serde(default)]
    pub data: BTreeMap<String, String>,
}

fn level_of(event: &DeviceEvent) -> ActivityLevel {
    match event.severity() {
        "error" => ActivityLevel::Error,
        "warning" => ActivityLevel::Warning,
        "success" => ActivityLevel::Success,
        _ => ActivityLevel::Info,
    }
}

impl HistoryEntry {
    fn from_event(event: &DeviceEvent) -> Self {
        Self {
            at_ms: event.when_ms(),
            level: level_of(event),
            kind: event.kind.clone(),
            source: event.source,
            origin: HistoryOrigin::Board,
            message: event.message.clone(),
            boot_id: Some(event.boot_id.clone()).filter(|id| !id.is_empty()),
            data: event.data.clone(),
        }
    }

    fn from_activity(entry: &ActivityEntry) -> Self {
        Self {
            at_ms: entry.at_ms,
            level: entry.level,
            kind: serde_json::to_value(entry.kind)
                .ok()
                .and_then(|kind| kind.as_str().map(str::to_string))
                .unwrap_or_default(),
            source: EventSource::Atlas,
            origin: HistoryOrigin::Atlas,
            message: entry.message.clone(),
            boot_id: None,
            data: BTreeMap::new(),
        }
    }
}

/// Places the page's events in this computer's time: the answering boot's
/// clock offset is measured (and kept by boot id); an event of a boot whose
/// offset is known, and off by more than [`CLOCK_SLACK_S`], gets `at_ms`.
fn place_in_time(page: &mut EventPage, offsets: &mut std::collections::HashMap<String, i64>) {
    if let (Some(time), Some(boot)) = (page.time, &page.boot_id) {
        let here = i64::try_from(now_ms() / 1000).unwrap_or(i64::MAX);
        offsets.insert(boot.clone(), time - here);
    }
    for event in &mut page.events {
        if let Some(offset) = offsets.get(&event.boot_id)
            && offset.abs() > CLOCK_SLACK_S
        {
            event.at_ms = u64::try_from(event.t - offset)
                .ok()
                .map(|s| s.saturating_mul(1000));
        }
    }
}

/// The order of a board's events: by seq once the board numbers them (its
/// earlier, unnumbered events first, by time).
fn order(event: &DeviceEvent) -> (u8, i64) {
    match event.seq {
        Some(seq) => (1, i64::try_from(seq).unwrap_or(i64::MAX)),
        None => (0, event.t),
    }
}

/// Adds the events `log` doesn't have yet, keeps it ordered (see [`order`])
/// and bounded, and returns the ones added.
pub(crate) fn merge(
    log: &mut VecDeque<DeviceEvent>,
    incoming: Vec<DeviceEvent>,
) -> Vec<DeviceEvent> {
    let mut added = Vec::new();
    for event in incoming {
        if log.iter().any(|kept| kept.same_as(&event))
            || added.iter().any(|new: &DeviceEvent| new.same_as(&event))
        {
            continue;
        }
        added.push(event);
    }
    if added.is_empty() {
        return added;
    }
    let mut all: Vec<DeviceEvent> = log.drain(..).chain(added.iter().cloned()).collect();
    // Stable: unnumbered events of one second keep the order the board
    // wrote them in.
    all.sort_by_key(order);
    let excess = all.len().saturating_sub(BOARD_EVENTS_LIMIT);
    log.extend(all.into_iter().skip(excess));
    added
}

/// Atlas's entries and the board's events for one device, newest first.
pub(crate) fn timeline(
    activity: &[ActivityEntry],
    events: &[DeviceEvent],
    limit: usize,
) -> Vec<HistoryEntry> {
    let mut all: Vec<HistoryEntry> = activity
        .iter()
        .map(HistoryEntry::from_activity)
        .chain(events.iter().map(HistoryEntry::from_event))
        .collect();
    // Both lists are oldest first: reversed, a stable sort keeps the newer of
    // two entries at the same time first.
    all.reverse();
    all.sort_by_key(|entry| std::cmp::Reverse(entry.at_ms));
    all.truncate(limit);
    all
}

/// Who, in words, for the fleet history.
fn by_whom(source: EventSource) -> &'static str {
    match source {
        EventSource::Atlas => "from Atlas",
        EventSource::Orion => "through Orion",
        EventSource::Local => "on the board",
        EventSource::Unknown => "",
    }
}

/// The fleet-history line for a new board event, when someone other than
/// Atlas caused something that matters.
pub(crate) fn notable(record: &DeviceRecord, event: &DeviceEvent) -> Option<ActivityEntry> {
    if event.source == EventSource::Atlas {
        return None;
    }
    let kind = event.kind.as_str();
    // Every step of an update, every restart and every failure, whoever
    // caused it.
    let matters = matches!(
        kind,
        "update.stage"
            | "update.download"
            | "update.staged"
            | "update.apply"
            | "update.apply-failed"
            | "update.apply-interrupted"
            | "update.confirmed"
            | "update.rolled-back"
            | "update.trial-failed"
            | "update.link-bad"
            | "update.link-fallback"
            | "update.cancelled"
            | "update.rollback"
            | "update.rollback-failed"
            | "update.failed"
            | "boot"
            | "reboot"
            | "ssh.keys"
            | "usb-boot"
            | "power-off"
    );
    if !matters {
        return None;
    }
    let who = by_whom(event.source);
    let message = if who.is_empty() {
        format!("{}: {}", record.display_name(), event.message)
    } else {
        format!("{}: {} ({who})", record.display_name(), event.message)
    };
    Some(ActivityEntry::about(
        record,
        ActivityKind::DeviceEvent,
        level_of(event),
        message,
    ))
}

impl Atlas {
    /// What the device is doing now and how it is. Also fetches its new
    /// events, so its history is current.
    pub async fn device_status(&self, key: &DeviceKey) -> Result<DeviceStatus, CoreError> {
        let (live, record) = self.live_device(key)?;
        let status = live.capabilities.status.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "status",
        })?;
        let current = status.status(&record.identity).await?;
        let _ = self.sync_device_events(key).await;
        Ok(current)
    }

    /// Fetches the board's new events and merges them: after the last seq
    /// fetched when the board numbers its events (pages until caught up),
    /// else since the newest kept time. Returns how many were new.
    pub async fn sync_device_events(&self, key: &DeviceKey) -> Result<usize, CoreError> {
        let (live, record) = self.live_device(key)?;
        let Some(status) = live.capabilities.status else {
            return Ok(0);
        };
        let board = board_of(&record);
        let (cursor, since, first) = {
            let mut state = self.inner.state();
            state.events_synced.insert(board.clone(), now_ms());
            let log = state.board_events.get(&board);
            let kept_seq = log.and_then(|log| log.iter().filter_map(|event| event.seq).max());
            let since = log.and_then(|log| log.back()).map(|event| event.t);
            let first = log.is_none_or(VecDeque::is_empty);
            let cursor = state.event_cursor.get(&board).copied().or(kept_seq);
            (cursor, since, first)
        };
        let mut query = match cursor {
            Some(after) => EventQuery {
                since: None,
                after_seq: Some(after),
                limit: PAGE,
            },
            None => EventQuery {
                since,
                after_seq: None,
                limit: if since.is_some() {
                    PAGE
                } else {
                    BOARD_EVENTS_LIMIT
                },
            },
        };
        let mut added = Vec::new();
        for _ in 0..MAX_PAGES {
            let mut page = status.event_page(&record.identity, query).await?;
            // A board whose counter started over (its /data was replaced)
            // numbers from 1 again: start from there.
            if let (Some(after), Some(newest)) = (query.after_seq, page.newest_seq)
                && newest < after
                && after > 0
            {
                query.after_seq = Some(0);
                continue;
            }
            let fetched = page.events.len();
            let newest_seen = page.events.iter().filter_map(|event| event.seq).max();
            {
                let mut state = self.inner.state();
                place_in_time(&mut page, &mut state.clock_offsets);
                let log = state.board_events.entry(board.clone()).or_default();
                added.extend(merge(log, page.events));
                if let Some(seq) = newest_seen.or(page.newest_seq) {
                    let cursor = state.event_cursor.entry(board.clone()).or_insert(seq);
                    *cursor = (*cursor).max(seq);
                    if query.after_seq == Some(0) {
                        *cursor = seq;
                    }
                }
            }
            match (query.after_seq, newest_seen) {
                (Some(_), Some(last)) if fetched >= query.limit => query.after_seq = Some(last),
                _ => break,
            }
        }
        if added.is_empty() {
            return Ok(0);
        }
        // A board's first sync brings its past: that is history, not news.
        if !first {
            let entries: Vec<ActivityEntry> = added
                .iter()
                .filter_map(|event| notable(&record, event))
                .collect();
            self.inner.record_activity(entries);
        }
        self.inner
            .events
            .emit(Event::DeviceHistory { key: key.clone() });
        self.inner.persist();
        Ok(added.len())
    }

    /// The device's history, newest first: Atlas's entries for it and its
    /// board's events (whichever way the board was seen when they happened).
    pub fn device_history(&self, key: &DeviceKey, limit: usize) -> Vec<HistoryEntry> {
        let state = self.inner.state();
        let activity: Vec<ActivityEntry> = state
            .activity
            .iter()
            .filter(|entry| entry.device.as_ref() == Some(key))
            .cloned()
            .collect();
        let events: Vec<DeviceEvent> = state
            .inventory
            .get(key)
            .map(board_of)
            .and_then(|board| state.board_events.get(&board))
            .map(|log| log.iter().cloned().collect())
            .unwrap_or_default();
        timeline(&activity, &events, limit.clamp(1, 2000))
    }

    /// After a scan: fetch new events from boards not synced for a while.
    pub(crate) fn start_due_event_syncs(&self) {
        let now = now_ms();
        let due: Vec<DeviceKey> = {
            let state = self.inner.state();
            state
                .live
                .iter()
                .filter(|(_, live)| live.capabilities.status.is_some())
                .filter_map(|(key, _)| {
                    let record = state.inventory.get(key)?;
                    let last = state
                        .events_synced
                        .get(&board_of(record))
                        .copied()
                        .unwrap_or(0);
                    (now.saturating_sub(last) >= SYNC_EVERY.as_millis() as u64).then(|| key.clone())
                })
                .collect()
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        for key in due {
            let atlas = self.clone();
            runtime.spawn(async move {
                let _ = atlas.sync_device_events(&key).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(t: i64, kind: &str, source: EventSource) -> DeviceEvent {
        DeviceEvent {
            t,
            boot_id: "b1".into(),
            kind: kind.into(),
            source,
            message: format!("{kind} at {t}"),
            data: BTreeMap::new(),
            seq: None,
            uptime_s: None,
            at_ms: None,
        }
    }

    #[test]
    fn merging_skips_events_already_kept() {
        let mut log = VecDeque::new();
        let added = merge(
            &mut log,
            vec![
                event(1, "boot", EventSource::Local),
                event(2, "update.stage", EventSource::Orion),
            ],
        );
        assert_eq!(added.len(), 2);
        // The next fetch overlaps (since = the newest t) and repeats itself.
        let added = merge(
            &mut log,
            vec![
                event(2, "update.stage", EventSource::Orion),
                event(3, "update.staged", EventSource::Orion),
                event(3, "update.staged", EventSource::Orion),
            ],
        );
        assert_eq!(added.len(), 1);
        assert_eq!(log.len(), 3);
        // Same second and kind, another boot: a different event.
        let mut other = event(3, "update.staged", EventSource::Orion);
        other.boot_id = "b2".into();
        assert_eq!(merge(&mut log, vec![other]).len(), 1);
    }

    #[test]
    fn the_log_is_ordered_and_bounded() {
        let mut log = VecDeque::new();
        merge(&mut log, vec![event(50, "late", EventSource::Local)]);
        merge(
            &mut log,
            (0..BOARD_EVENTS_LIMIT as i64 + 10)
                .map(|t| event(t, "n", EventSource::Local))
                .collect(),
        );
        assert_eq!(log.len(), BOARD_EVENTS_LIMIT);
        assert!(log.iter().zip(log.iter().skip(1)).all(|(a, b)| a.t <= b.t));
        assert_eq!(log.back().unwrap().t, BOARD_EVENTS_LIMIT as i64 + 9);
    }

    #[test]
    fn the_timeline_interleaves_atlas_and_the_board() {
        let activity = vec![ActivityEntry {
            at_ms: 2_500,
            kind: ActivityKind::ActionRun,
            level: ActivityLevel::Info,
            device: None,
            robot: None,
            message: "Restart on cam".into(),
        }];
        let events = vec![
            event(1, "boot", EventSource::Local),
            event(3, "update.confirmed", EventSource::Orion),
        ];
        let line = timeline(&activity, &events, 10);
        let kinds: Vec<(&str, EventSource, HistoryOrigin)> = line
            .iter()
            .map(|entry| (entry.kind.as_str(), entry.source, entry.origin))
            .collect();
        assert_eq!(
            kinds,
            [
                ("update.confirmed", EventSource::Orion, HistoryOrigin::Board),
                ("action-run", EventSource::Atlas, HistoryOrigin::Atlas),
                ("boot", EventSource::Local, HistoryOrigin::Board),
            ]
        );
        assert_eq!(line[0].level, ActivityLevel::Success);
        assert_eq!(timeline(&activity, &events, 1).len(), 1);
    }
}
