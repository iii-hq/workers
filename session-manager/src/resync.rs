//! Post-reload trigger resync.
//!
//! Subscribers (the console, the harness, ...) hold session and transcript
//! state they reconciled from the six `session::*` triggers — they do not poll.
//! When an adapter change swaps the [`SessionStore`] under them, nothing has
//! re-emitted those triggers, so an open view would stay stale until the next
//! mutation even though new events would now flow correctly.
//!
//! [`resync_triggers`] walks the NEW store read-only and replays its current
//! state through the local [`Emitter`]: `session::created` for every live
//! session, then one snapshot event per entry (`message-added` for a fresh
//! entry, `message-updated` for one that had been edited). Sessions that were
//! present before the swap but are gone after it are signalled with
//! `session::deleted`. Consumers reconcile by entry id and highest revision, so
//! a single snapshot per entry is enough to converge an open view.
//!
//! Delivery always goes through the local emitter (never a `RemotePublisher`):
//! a replay is local state, and on an fs-main instance the emitter still fans
//! each envelope out to attached bridges, so they resync too.

use std::collections::HashSet;

use crate::events::{
    EmittableEvent, Emitter, MessageAddedEvent, MessageUpdatedEvent, SessionDeletedEvent,
    SessionEvent,
};
use crate::service::{created_event, Clock, SystemClock};
use crate::store::SessionStore;
use crate::types::{CustomPayload, JsonMap, SessionEntry, SessionMeta};

/// What a resync replayed, for an operator-visible log line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResyncStats {
    /// Live sessions re-announced via `session::created`.
    pub sessions: usize,
    /// Entries replayed via `session::message-added` / `message-updated`.
    pub entries: usize,
    /// Sessions signalled gone via `session::deleted`.
    pub deleted: usize,
    /// Total events emitted (`sessions + entries + deleted`).
    pub events: usize,
}

/// Replay the new store's current state through `emitter`, plus a
/// `session::deleted` for every id in `old_metas` that is absent from the new
/// store. Read-only: it never mutates the store.
pub async fn resync_triggers(
    old_metas: &[SessionMeta],
    store: &dyn SessionStore,
    emitter: &Emitter,
) -> Result<ResyncStats, String> {
    let new_metas = store
        .list_metas()
        .await
        .map_err(|e| format!("resync: list_metas on the new store failed: {e}"))?;
    let new_ids: HashSet<&str> = new_metas.iter().map(|m| m.session_id.as_str()).collect();

    let mut events: Vec<EmittableEvent> = Vec::new();
    let mut stats = ResyncStats::default();

    // 1. Sessions that existed before the swap but not after.
    let now = SystemClock.now_ms();
    for old in old_metas {
        if !new_ids.contains(old.session_id.as_str()) {
            events.push(EmittableEvent {
                event: SessionEvent::Deleted(SessionDeletedEvent {
                    session_id: old.session_id.clone(),
                    timestamp: now,
                }),
                session_metadata: old.metadata.clone(),
            });
            stats.deleted += 1;
        }
    }

    // 2/3. Recreate every current session, then replay its entries so open
    //      transcripts reconcile in place. `created` precedes the session's
    //      entries because `emit_all` preserves order.
    for meta in &new_metas {
        events.push(created_event(meta));
        stats.sessions += 1;
        let entries = store
            .list_entries(&meta.session_id)
            .await
            .map_err(|e| format!("resync: list_entries({}) failed: {e}", meta.session_id))?;
        for entry in &entries {
            events.push(entry_event(&meta.session_id, entry, meta.metadata.clone()));
            stats.entries += 1;
        }
    }

    stats.events = events.len();
    emitter.emit_all(&events).await;
    Ok(stats)
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
            message: message.clone(),
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
            message: Some(message.clone()),
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
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use iii_sdk::trigger::TriggerConfig;
    use serde_json::{json, Value};

    use crate::events::{EventDeliverer, EventKind, TriggerSets};
    use crate::store::FsStore;
    use crate::types::{AgentMessage, SessionStatus};

    /// Records the (trigger_type, payload) of every delivery for assertions.
    struct RecordingDeliverer {
        seen: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        async fn deliver(&self, trigger_type: &str, _function_id: &str, payload: Value) {
            self.seen
                .lock()
                .unwrap()
                .push((trigger_type.to_string(), payload));
        }
    }

    fn bind_all(sets: &TriggerSets) {
        for kind in EventKind::all() {
            sets.for_kind(kind)
                .add(TriggerConfig {
                    id: format!("b-{}", kind.trigger_type()),
                    function_id: "test::recv".to_string(),
                    config: json!({}),
                    metadata: None,
                })
                .unwrap();
        }
    }

    fn meta(session_id: &str) -> SessionMeta {
        SessionMeta {
            session_id: session_id.to_string(),
            title: "t".to_string(),
            description: "d".to_string(),
            status: SessionStatus::Idle,
            status_reason: None,
            metadata: None,
            forked_from: None,
            draft: None,
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
            message: AgentMessage::User {
                content: vec![],
                timestamp: 2,
            },
        }
    }

    #[tokio::test]
    async fn replays_created_added_updated_and_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FsStore::new(tmp.path()).unwrap();

        // New store: one session with a fresh entry (rev 0) and an edited one (rev 3).
        store.put_meta(&meta("s1")).await.unwrap();
        store.put_entry("s1", &message("e1", 0)).await.unwrap();
        store.put_entry("s1", &message("e2", 3)).await.unwrap();

        let recorder = Arc::new(RecordingDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let sets = TriggerSets::new();
        bind_all(&sets);
        let emitter = Emitter::new(sets, recorder.clone());

        // Pre-swap there was also s0, which is gone from the new store.
        let old = vec![meta("s0"), meta("s1")];
        let stats = resync_triggers(&old, &store, &emitter).await.unwrap();

        assert_eq!(stats.deleted, 1, "s0 vanished across the swap");
        assert_eq!(stats.sessions, 1, "s1 re-announced");
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.events, 4);

        let seen = recorder.seen.lock().unwrap();
        let kinds: Vec<&str> = seen.iter().map(|(t, _)| t.as_str()).collect();
        assert!(kinds.contains(&"session::deleted"));
        assert!(kinds.contains(&"session::created"));
        assert!(kinds.contains(&"session::message-added"));
        assert!(kinds.contains(&"session::message-updated"));
    }

    #[tokio::test]
    async fn empty_store_with_no_old_sessions_emits_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FsStore::new(tmp.path()).unwrap();
        let recorder = Arc::new(RecordingDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let sets = TriggerSets::new();
        bind_all(&sets);
        let emitter = Emitter::new(sets, recorder.clone());

        let stats = resync_triggers(&[], &store, &emitter).await.unwrap();
        assert_eq!(stats.events, 0);
        assert!(recorder.seen.lock().unwrap().is_empty());
    }
}
