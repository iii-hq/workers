//! A watch lives only while someone listens to it.
//!
//! Observers of `github::pr::on-event` are the listeners. When the last
//! observer that matched a watch goes away (its chat was deleted, or it was
//! disposed), the watch is stopped; cleanup then releases its tunnel lease and deletes the
//! repository hook once no live watch remains there. The quick-tunnel stops
//! with its last lease.

use super::types::{Data, EventFilter, Watch, WatchState};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeSet;

/// Whether an observer with this filter receives events of this watch.
/// Categories are ignored: a narrowed observer still listens to the watch.
pub fn listens(filter: &EventFilter, watch_id: &str, watch: &Watch) -> bool {
    filter.watch_id.as_ref().is_none_or(|v| v == watch_id)
        && filter
            .repo
            .as_ref()
            .is_none_or(|v| v.eq_ignore_ascii_case(&watch.spec.repo))
        && filter.number.is_none_or(|v| v == watch.spec.number)
}

/// Live watches that a removed observer listened to and that no remaining
/// observer listens to. A watch nobody ever observed is never selected: only
/// losing a listener can orphan a watch.
pub fn orphaned(data: &Data, removed: &[EventFilter]) -> Vec<String> {
    data.watches
        .iter()
        .filter(|(id, w)| w.live() && removed.iter().any(|f| listens(f, id, w)))
        .filter(|(id, w)| !data.subscribers.values().any(|s| listens(&s.filter, id, w)))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Local subscribers to drop: those without an observer in `observed` that
/// were also absent at the previous reconciliation (`absent_before`), so an
/// observer the engine has not announced yet after a restart is never
/// dropped. `known` must be read before `observed`, so an observer added
/// meanwhile is never dropped. Returns the subscribers to drop and the ones
/// absent for the first time, to check again next time.
pub fn stale(
    known: &BTreeSet<String>,
    observed: &BTreeSet<String>,
    absent_before: &BTreeSet<String>,
) -> (Vec<String>, BTreeSet<String>) {
    let (stale, absent): (Vec<String>, Vec<String>) = known
        .difference(observed)
        .cloned()
        .partition(|id| absent_before.contains(id));
    (stale, absent.into_iter().collect())
}

/// Mark the watches a removed observer left without listeners (an earlier mark
/// is kept). They are not stopped yet: a one-shot wake is disposed right
/// after it fires and observed again at the end of its turn.
pub fn mark_orphans(data: &mut Data, removed: &[EventFilter], now: DateTime<Utc>) -> Vec<String> {
    let ids = orphaned(data, removed);
    for id in &ids {
        if let Some(w) = data.watches.get_mut(id) {
            w.orphaned_at.get_or_insert(now);
        }
    }
    ids
}

/// Stop marked watches still without listeners after `grace`; clear the mark
/// of those an observer listens to again. Returns the stopped ids.
pub fn expire_orphans(data: &mut Data, now: DateTime<Utc>, grace: Duration) -> Vec<String> {
    let marked: Vec<String> = data
        .watches
        .iter()
        .filter(|(_, w)| w.live() && w.orphaned_at.is_some())
        .map(|(id, _)| id.clone())
        .collect();
    let mut stopped = Vec::new();
    for id in marked {
        let Some(w) = data.watches.get(&id) else {
            continue;
        };
        let listened = data
            .subscribers
            .values()
            .any(|s| listens(&s.filter, &id, w));
        let since = w.orphaned_at;
        let Some(w) = data.watches.get_mut(&id) else {
            continue;
        };
        if listened {
            w.orphaned_at = None;
        } else if since.is_some_and(|t| now - t >= grace) {
            w.status = WatchState::Stopped;
            w.orphaned_at = None;
            stopped.push(id);
        }
    }
    stopped
}
