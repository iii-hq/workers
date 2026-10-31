//! The message paths gate on observation before building an event
//! (contract §1.2 "Gate on observation"; row 81, `session::on-message-updated`,
//! the hottest source). `session::append`, `session::append-many` and
//! `session::update-message` ask the sink whether it wants the event first:
//! when it does not, the mutation is stored and no event is built; when it
//! does, the event carries the stored message and the session metadata.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use session_manager::config::WorkerConfig;
use session_manager::events::{EmittableEvent, EventKind, EventSink, SessionEvent};
use session_manager::functions::{append, append_many, create, get_message, update_message, Deps};
use session_manager::service::SessionService;
use session_manager::store::FsStore;

/// A sink that wants events or not, and records what it was asked and what
/// it was handed.
struct GateSink {
    wants: bool,
    asked: Mutex<Vec<EventKind>>,
    published: Mutex<Vec<EmittableEvent>>,
}

#[async_trait]
impl EventSink for GateSink {
    fn wants(&self, kind: EventKind) -> bool {
        self.asked.lock().unwrap().push(kind);
        self.wants
    }

    async fn publish_all(&self, events: &[EmittableEvent]) {
        self.published.lock().unwrap().extend_from_slice(events);
    }
}

struct Stack {
    deps: Deps,
    sink: Arc<GateSink>,
    _dir: tempfile::TempDir,
}

fn stack(wants: bool) -> Stack {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(FsStore::new(dir.path()).expect("open FsStore"));
    let service = Arc::new(SessionService::new(store, &WorkerConfig::default()));
    let sink = Arc::new(GateSink {
        wants,
        asked: Mutex::new(Vec::new()),
        published: Mutex::new(Vec::new()),
    });
    Stack {
        deps: Deps {
            service,
            sink: sink.clone(),
        },
        sink,
        _dir: dir,
    }
}

fn req<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("request deserializes")
}

impl Stack {
    /// A session with metadata, and nothing recorded yet.
    async fn session(&self) -> String {
        let created = create::handle(
            &self.deps,
            req(json!({ "title": "gate", "metadata": { "owner": "u_1" } })),
        )
        .await
        .expect("create");
        self.sink.asked.lock().unwrap().clear();
        self.sink.published.lock().unwrap().clear();
        created.session_id
    }

    async fn stored(&self, session_id: &str, entry_id: &str) -> Value {
        let found = get_message::handle(
            &self.deps,
            req(json!({ "session_id": session_id, "entry_id": entry_id })),
        )
        .await
        .expect("get-message")
        .expect("entry is stored");
        serde_json::to_value(found.entry).unwrap()
    }

    fn asked(&self) -> Vec<EventKind> {
        self.sink.asked.lock().unwrap().clone()
    }

    fn published(&self) -> Vec<EmittableEvent> {
        self.sink.published.lock().unwrap().clone()
    }
}

fn assistant(text: &str) -> Value {
    json!({
        "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 1
    })
}

#[tokio::test]
async fn an_unwanted_message_event_is_never_built() {
    let s = stack(false);
    let session_id = s.session().await;

    let appended = append::handle(
        &s.deps,
        req(json!({ "session_id": session_id, "entry_id": "reply-1", "message": assistant("") })),
    )
    .await
    .expect("append");
    assert_eq!(appended.entry_id, "reply-1");

    let updated = update_message::handle(
        &s.deps,
        req(json!({
            "session_id": session_id, "entry_id": "reply-1",
            "content": [{ "type": "text", "text": "It works." }]
        })),
    )
    .await
    .expect("update-message");
    assert!(updated.updated);
    assert_eq!(updated.revision, 1);

    let many = append_many::handle(
        &s.deps,
        req(json!({ "session_id": session_id, "messages": [assistant("a"), assistant("b")] })),
    )
    .await
    .expect("append-many");
    assert_eq!(many.entry_ids.len(), 2);

    assert_eq!(
        s.asked(),
        vec![
            EventKind::MessageAdded,
            EventKind::MessageUpdated,
            EventKind::MessageAdded
        ],
        "each mutation asks the sink before building its event"
    );
    assert!(
        s.published().is_empty(),
        "no event is built for a sink that does not want it: {:#?}",
        s.published()
    );

    // The mutations themselves are stored in full.
    let reply = s.stored(&session_id, "reply-1").await;
    assert_eq!(reply["revision"], 1);
    assert_eq!(reply["message"]["content"][0]["text"], "It works.");
    for entry_id in &many.entry_ids {
        let entry = s.stored(&session_id, entry_id).await;
        assert_eq!(entry["message"]["role"], "assistant");
    }
}

#[tokio::test]
async fn a_wanted_message_event_carries_the_stored_message_and_metadata() {
    let s = stack(true);
    let session_id = s.session().await;

    append::handle(
        &s.deps,
        req(json!({
            "session_id": session_id, "entry_id": "reply-1", "message": assistant(""),
            "origin": { "turn_id": "t_1" }
        })),
    )
    .await
    .expect("append");
    update_message::handle(
        &s.deps,
        req(json!({
            "session_id": session_id, "entry_id": "reply-1",
            "content": [{ "type": "text", "text": "It works." }],
            "origin": { "turn_id": "t_1" }
        })),
    )
    .await
    .expect("update-message");
    append_many::handle(
        &s.deps,
        req(json!({ "session_id": session_id, "messages": [assistant("a"), assistant("b")] })),
    )
    .await
    .expect("append-many");

    let published = s.published();
    assert_eq!(published.len(), 4, "{published:#?}");
    for ev in &published {
        assert_eq!(ev.event.session_id(), session_id);
        assert_eq!(
            ev.session_metadata.as_ref().map(|m| m["owner"].clone()),
            Some(json!("u_1"))
        );
    }

    let SessionEvent::MessageAdded(added) = &published[0].event else {
        panic!("append publishes message-added: {:?}", published[0]);
    };
    assert_eq!(added.entry_id, "reply-1");
    assert_eq!(added.origin.as_ref().unwrap()["turn_id"], "t_1");
    assert!(added.message.is_some() && added.custom.is_none());

    let SessionEvent::MessageUpdated(delta) = &published[1].event else {
        panic!(
            "update-message publishes message-updated: {:?}",
            published[1]
        );
    };
    assert_eq!(delta.entry_id, "reply-1");
    assert_eq!(delta.revision, 1);
    assert_eq!(
        serde_json::to_value(&delta.message).unwrap(),
        s.stored(&session_id, "reply-1").await["message"],
        "the event's message is the stored message"
    );

    let parents: Vec<Option<String>> = published[2..]
        .iter()
        .map(|ev| match &ev.event {
            SessionEvent::MessageAdded(e) => e.parent_id.clone(),
            other => panic!("append-many publishes message-added: {other:?}"),
        })
        .collect();
    let SessionEvent::MessageAdded(first) = &published[2].event else {
        unreachable!()
    };
    assert_eq!(
        parents,
        vec![Some("reply-1".to_string()), Some(first.entry_id.clone())],
        "append-many chains its entries"
    );
}

/// An idempotent re-append stores nothing and builds no event, whatever the
/// sink wants.
#[tokio::test]
async fn a_repeated_append_builds_no_event() {
    let s = stack(true);
    let session_id = s.session().await;
    let first = req::<append::AppendRequest>(
        json!({ "session_id": session_id, "entry_id": "u-1", "message": assistant("x") }),
    );
    append::handle(&s.deps, first.clone())
        .await
        .expect("append");
    append::handle(&s.deps, first).await.expect("re-append");
    assert_eq!(s.published().len(), 1, "{:#?}", s.published());
}
