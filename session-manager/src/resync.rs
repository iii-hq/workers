//! Post-reload resync of the session event observers.
//!
//! Observers (the console, the harness, ...) hold session and transcript
//! state they reconciled from the six `session::on-*` event functions; they do
//! not poll. When an adapter change swaps the [`SessionStore`] under them,
//! nothing has called those functions for the new store, so an open view would
//! stay stale until the next mutation even though new events would flow
//! correctly.
//!
//! [`resync_observers`] walks the NEW store read-only and replays its current
//! state through the local [`Emitter`]: `session::on-created` for every live
//! session, then one snapshot event per entry (`session::on-message-added` for
//! a fresh entry, `session::on-message-updated` for one that had been edited).
//! Sessions that were present before the swap but are gone after it are
//! signalled with `session::on-deleted`. Consumers reconcile by entry id and
//! highest revision, so a single snapshot per entry is enough to converge an
//! open view.
//!
//! The replay gates on observation before it builds anything, as the message
//! paths do: it asks the emitter whether it wants each kind of event
//! ([`Emitter::wants`]) and builds only those events. A function nothing
//! observes costs no event; entries are read only while
//! `session::on-message-added` or `session::on-message-updated` is wanted; and
//! a resync that nothing would receive reads nothing from the store.
//!
//! Delivery always goes through the local emitter (never a `RemotePublisher`):
//! a replay is local state, and on an fs-main instance the emitter still calls
//! `session::store::on-events` with each envelope, so attached bridges resync
//! too.

use std::collections::HashSet;

use crate::events::{
    EmittableEvent, Emitter, EventKind, MessageAddedEvent, MessageUpdatedEvent,
    SessionDeletedEvent, SessionEvent,
};
use crate::service::{created_event, Clock, SystemClock};
use crate::store::SessionStore;
use crate::types::{CustomPayload, JsonMap, SessionEntry, SessionMeta};

/// What a resync replayed, for an operator-visible log line. Each count
/// covers the events the replay built, which are the events of the functions
/// the emitter wanted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResyncStats {
    /// Live sessions re-announced via `session::on-created`.
    pub sessions: usize,
    /// Entries replayed via `session::on-message-added` /
    /// `session::on-message-updated`.
    pub entries: usize,
    /// Sessions signalled gone via `session::on-deleted`.
    pub deleted: usize,
    /// Total events emitted (`sessions + entries + deleted`).
    pub events: usize,
}

/// Which of the replayed kinds of event the emitter wants, asked once per
/// resync.
#[derive(Debug, Clone, Copy)]
struct Wanted {
    created: bool,
    added: bool,
    updated: bool,
    deleted: bool,
}

impl Wanted {
    fn of(emitter: &Emitter) -> Self {
        Self {
            created: emitter.wants(EventKind::Created),
            added: emitter.wants(EventKind::MessageAdded),
            updated: emitter.wants(EventKind::MessageUpdated),
            deleted: emitter.wants(EventKind::Deleted),
        }
    }

    fn entries(&self) -> bool {
        self.added || self.updated
    }

    fn any(&self) -> bool {
        self.created || self.entries() || self.deleted
    }

    fn kind(&self, kind: EventKind) -> bool {
        match kind {
            EventKind::Created => self.created,
            EventKind::MessageAdded => self.added,
            EventKind::MessageUpdated => self.updated,
            EventKind::Deleted => self.deleted,
            EventKind::StatusChanged | EventKind::MetaUpdated => false,
        }
    }
}

/// Replay the new store's current state through `emitter`, plus a
/// `session::on-deleted` for every id in `old_metas` that is absent from the
/// new store. Read-only: it never mutates the store. Builds only the events
/// the emitter wants; when it wants none, it reads nothing and sends nothing.
pub async fn resync_observers(
    old_metas: &[SessionMeta],
    store: &dyn SessionStore,
    emitter: &Emitter,
) -> Result<ResyncStats, String> {
    let wanted = Wanted::of(emitter);
    let mut stats = ResyncStats::default();
    if !wanted.any() {
        return Ok(stats);
    }

    let new_metas = store
        .list_metas()
        .await
        .map_err(|e| format!("resync: list_metas on the new store failed: {e}"))?;
    let new_ids: HashSet<&str> = new_metas.iter().map(|m| m.session_id.as_str()).collect();

    let mut events: Vec<EmittableEvent> = Vec::new();

    // 1. Sessions that existed before the swap but not after.
    let now = SystemClock.now_ms();
    let gone = old_metas
        .iter()
        .filter(|old| wanted.deleted && !new_ids.contains(old.session_id.as_str()));
    for old in gone {
        events.push(EmittableEvent {
            event: SessionEvent::Deleted(SessionDeletedEvent {
                session_id: old.session_id.clone(),
                timestamp: now,
            }),
            session_metadata: old.metadata.clone(),
        });
        stats.deleted += 1;
    }

    // 2/3. Recreate every current session, then replay its entries so open
    //      transcripts reconcile in place. `created` precedes the session's
    //      entries because `emit_all` preserves order.
    for meta in &new_metas {
        if wanted.created {
            events.push(created_event(meta));
            stats.sessions += 1;
        }
        if !wanted.entries() {
            continue;
        }
        let entries = store
            .list_entries(&meta.session_id)
            .await
            .map_err(|e| format!("resync: list_entries({}) failed: {e}", meta.session_id))?;
        for entry in entries.iter().filter(|e| wanted.kind(entry_kind(e))) {
            events.push(entry_event(&meta.session_id, entry, meta.metadata.clone()));
            stats.entries += 1;
        }
    }

    stats.events = events.len();
    emitter.emit_all(&events).await;
    Ok(stats)
}

/// The kind of the snapshot event [`entry_event`] builds for `entry`: an
/// update for a message edited since creation, an add otherwise.
fn entry_kind(entry: &SessionEntry) -> EventKind {
    match entry {
        SessionEntry::Message { revision, .. } if *revision > 0 => EventKind::MessageUpdated,
        SessionEntry::Message { .. } | SessionEntry::Custom { .. } => EventKind::MessageAdded,
    }
}

/// The single snapshot event that replays one stored entry.
fn entry_event(
    session_id: &str,
    entry: &SessionEntry,
    session_metadata: Option<JsonMap>,
) -> EmittableEvent {
    let event = match entry {
        // A message edited since creation: replay only the latest snapshot
        // (consumers keep the highest revision per entry).
        SessionEntry::Message {
            id,
            timestamp,
            revision,
            origin,
            message,
            ..
        } if *revision > 0 => SessionEvent::MessageUpdated(MessageUpdatedEvent {
            session_id: session_id.to_string(),
            entry_id: id.clone(),
            message: (**message).clone(),
            revision: *revision,
            origin: origin.clone(),
            timestamp: *timestamp,
        }),
        // A freshly-added message (revision 0).
        SessionEntry::Message {
            id,
            parent_id,
            timestamp,
            origin,
            message,
            ..
        } => SessionEvent::MessageAdded(MessageAddedEvent {
            session_id: session_id.to_string(),
            entry_id: id.clone(),
            parent_id: parent_id.clone(),
            message: Some((**message).clone()),
            custom: None,
            origin: origin.clone(),
            timestamp: *timestamp,
        }),
        // Custom bookkeeping entry: always replayed as an add (update-message
        // never applies to custom entries, so revision stays 0).
        SessionEntry::Custom {
            id,
            parent_id,
            timestamp,
            origin,
            custom_type,
            data,
            ..
        } => SessionEvent::MessageAdded(MessageAddedEvent {
            session_id: session_id.to_string(),
            entry_id: id.clone(),
            parent_id: parent_id.clone(),
            message: None,
            custom: Some(CustomPayload {
                custom_type: custom_type.clone(),
                data: data.clone(),
            }),
            origin: origin.clone(),
            timestamp: *timestamp,
        }),
    };
    EmittableEvent {
        event,
        session_metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::Value;

    use crate::events::recording::RecordingDeliverer;
    use crate::events::{ON_CREATED_FN, ON_MESSAGE_UPDATED_FN, STORE_ON_EVENTS_FN};
    use crate::store::counting::CountingStore;
    use crate::store::FsStore;
    use crate::types::{AgentMessage, SessionKind, SessionStatus};

    fn meta(session_id: &str) -> SessionMeta {
        SessionMeta {
            session_id: session_id.to_string(),
            title: "t".to_string(),
            description: "d".to_string(),
            status: SessionStatus::Idle,
            status_reason: None,
            kind: SessionKind::User,
            metadata: None,
            forked_from: None,
            draft: None,
            draft_attachments: None,
            created_at: 1,
            updated_at: 1,
            message_count: 0,
        }
    }

    fn message(id: &str, revision: u64) -> SessionEntry {
        SessionEntry::Message {
            id: id.to_string(),
            parent_id: None,
            timestamp: 2,
            revision,
            origin: None,
            message: Box::new(AgentMessage::User {
                content: vec![],
                timestamp: 2,
            }),
        }
    }

    fn custom(id: &str) -> SessionEntry {
        SessionEntry::Custom {
            id: id.to_string(),
            parent_id: None,
            timestamp: 2,
            revision: 0,
            origin: None,
            custom_type: "compaction".to_string(),
            data: Value::Null,
        }
    }

    /// The store the swap lands on: s1 with a fresh entry (rev 0) and an
    /// edited one (rev 3). Pre-swap there was also s0, which is gone.
    async fn swapped_store() -> (tempfile::TempDir, CountingStore, Vec<SessionMeta>) {
        let tmp = tempfile::tempdir().unwrap();
        let store = CountingStore::new(FsStore::new(tmp.path()).unwrap());
        store.put_meta(&meta("s1")).await.unwrap();
        store.put_entry("s1", &message("e1", 0)).await.unwrap();
        store.put_entry("s1", &message("e2", 3)).await.unwrap();
        (tmp, store, vec![meta("s0"), meta("s1")])
    }

    #[tokio::test]
    async fn replays_created_added_updated_and_deleted() {
        let (_tmp, store, old) = swapped_store().await;
        let recorder = RecordingDeliverer::observing_all();
        let emitter = Emitter::new(recorder.clone());

        let stats = resync_observers(&old, &store, &emitter).await.unwrap();

        assert_eq!(stats.deleted, 1, "s0 vanished across the swap");
        assert_eq!(stats.sessions, 1, "s1 re-announced");
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.events, 4);

        let functions = recorder.functions();
        assert!(functions.contains(&"session::on-deleted".to_string()));
        assert!(functions.contains(&"session::on-created".to_string()));
        assert!(functions.contains(&"session::on-message-added".to_string()));
        assert!(functions.contains(&"session::on-message-updated".to_string()));
    }

    /// Nothing observes any session event function, and no bridge observes
    /// the feed: the resync reads nothing, builds nothing and sends nothing.
    #[tokio::test]
    async fn an_unobserved_resync_reads_builds_and_sends_nothing() {
        let (_tmp, store, old) = swapped_store().await;
        for emitter_of in [Emitter::new, Emitter::feeding_bridges] {
            let recorder = RecordingDeliverer::observing(&[]);
            let emitter = emitter_of(recorder.clone());

            let stats = resync_observers(&old, &store, &emitter).await.unwrap();

            assert_eq!(stats, ResyncStats::default(), "no event was built");
            assert!(recorder.seen.lock().unwrap().is_empty(), "nothing was sent");
            assert_eq!(store.reads(), (0, 0), "the store was not read");
        }
    }

    /// Only `session::on-message-updated` is observed: the replay builds the
    /// edited entry's snapshot alone, with no created, added or deleted
    /// event.
    #[tokio::test]
    async fn a_resync_builds_only_the_observed_kinds() {
        let (_tmp, store, old) = swapped_store().await;
        let recorder = RecordingDeliverer::observing(&[ON_MESSAGE_UPDATED_FN]);
        let emitter = Emitter::new(recorder.clone());

        let stats = resync_observers(&old, &store, &emitter).await.unwrap();

        assert_eq!(
            stats,
            ResyncStats {
                sessions: 0,
                entries: 1,
                deleted: 0,
                events: 1,
            }
        );
        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, ON_MESSAGE_UPDATED_FN);
        assert_eq!(seen[0].1["entry_id"], "e2");
        assert_eq!(seen[0].1["revision"], 3);
    }

    /// Only `session::on-created` is observed: the replay re-announces the
    /// sessions and never lists their entries.
    #[tokio::test]
    async fn a_resync_for_created_alone_reads_no_entries() {
        let (_tmp, store, old) = swapped_store().await;
        let recorder = RecordingDeliverer::observing(&[ON_CREATED_FN]);
        let emitter = Emitter::new(recorder.clone());

        let stats = resync_observers(&old, &store, &emitter).await.unwrap();

        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.events, 1);
        assert_eq!(recorder.functions(), vec![ON_CREATED_FN.to_string()]);
        assert_eq!(store.reads(), (1, 0), "metas listed once, entries never");
    }

    /// On the main instance a bridge observing the feed wants every kind:
    /// each event goes to the feed, though no local observer has its function.
    #[tokio::test]
    async fn a_bridge_on_the_feed_receives_the_whole_replay() {
        let (_tmp, store, old) = swapped_store().await;
        let recorder = RecordingDeliverer::observing(&[STORE_ON_EVENTS_FN]);
        let emitter = Emitter::feeding_bridges(recorder.clone());

        let stats = resync_observers(&old, &store, &emitter).await.unwrap();

        assert_eq!(stats.events, 4);
        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().all(|(f, _)| f == STORE_ON_EVENTS_FN));
        let mut replayed: Vec<&str> = seen
            .iter()
            .map(|(_, envelope)| envelope["function_id"].as_str().unwrap())
            .collect();
        // Deleted first, then the session ahead of its entries; a store
        // lists entries in no particular order.
        replayed[2..].sort_unstable();
        assert_eq!(
            replayed,
            [
                "session::on-deleted",
                "session::on-created",
                "session::on-message-added",
                "session::on-message-updated",
            ]
        );
    }

    #[test]
    fn entry_kind_names_the_event_entry_event_builds() {
        for entry in [message("e1", 0), message("e2", 3), custom("c1")] {
            assert_eq!(
                entry_kind(&entry),
                entry_event("s1", &entry, None).event.kind(),
                "{entry:?}"
            );
        }
        assert_eq!(entry_kind(&message("e2", 3)), EventKind::MessageUpdated);
        assert_eq!(entry_kind(&custom("c1")), EventKind::MessageAdded);
    }

    #[tokio::test]
    async fn empty_store_with_no_old_sessions_emits_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FsStore::new(tmp.path()).unwrap();
        let recorder = RecordingDeliverer::observing_all();
        let emitter = Emitter::new(recorder.clone());

        let stats = resync_observers(&[], &store, &emitter).await.unwrap();
        assert_eq!(stats.events, 0);
        assert!(recorder.seen.lock().unwrap().is_empty());
    }
}
