//! `storage::on-object-created` / `storage::on-object-deleted`: each refuses
//! direct invocation (FORBIDDEN), its observers are admitted exactly like a
//! binding of the trigger type next to it, and an observer receives the
//! payload a binding receives from the same fan-out. No engine is needed:
//! the dispatcher only reaches the engine for bindings.

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use storage::triggers::dispatcher::{event_payload, EngineDispatcher, EventDispatcher};
use storage::triggers::handler::{ObjectCreatedHandler, WiredBuckets};
use storage::triggers::normalize::{EventKind, ObjectEventNormalized};
use storage::triggers::observe::{
    function_id, refuse_invocation, OnObjectHandler, ON_OBJECT_CREATED, ON_OBJECT_DELETED,
};
use storage::triggers::registry::{ObjectObserver, TriggerRegistry};
use tokio::sync::RwLock;

const KINDS: [EventKind; 2] = [EventKind::Created, EventKind::Deleted];

/// Stands in for the SDK observer: records what `obs.call` would carry and
/// answers with a fixed result.
struct Recorder {
    id: String,
    reply: Result<Value, Error>,
    seen: Mutex<Vec<Value>>,
}

impl Recorder {
    fn new(id: &str) -> Arc<Self> {
        Self::replying(id, Ok(Value::Null))
    }

    fn replying(id: &str, reply: Result<Value, Error>) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            reply,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl ObjectObserver for Recorder {
    fn id(&self) -> &str {
        &self.id
    }

    async fn deliver(&self, payload: &Value, _timeout: Duration) -> Result<Value, Error> {
        self.seen.lock().unwrap().push(payload.clone());
        self.reply.clone()
    }
}

fn wired(buckets: &[&str]) -> WiredBuckets {
    Arc::new(RwLock::new(
        buckets
            .iter()
            .map(|b| b.to_string())
            .collect::<HashSet<_>>(),
    ))
}

fn handler(kind: EventKind, registry: &Arc<TriggerRegistry>) -> OnObjectHandler {
    OnObjectHandler {
        kind,
        registry: registry.clone(),
        wired_buckets: wired(&["uploads"]),
        reconfigure_gate: Arc::new(RwLock::new(())),
    }
}

fn event(kind: EventKind, bucket: &str) -> ObjectEventNormalized {
    let at = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    ObjectEventNormalized {
        bucket: bucket.into(),
        key: "a/b.txt".into(),
        size: 42,
        etag: "etag-1".into(),
        content_type: Some("text/plain".into()),
        created_at: (kind == EventKind::Created).then_some(at),
        deleted_at: (kind == EventKind::Deleted).then_some(at),
        event_kind: kind,
        raw_event_id: "evt-1".into(),
    }
}

fn dispatcher(registry: &Arc<TriggerRegistry>) -> EngineDispatcher {
    EngineDispatcher::new(
        IIIClient::new("ws://stub-not-connected:0"),
        registry.clone(),
    )
}

#[test]
fn each_on_function_refuses_direct_invocation() {
    assert_eq!(
        function_id(EventKind::Created),
        "storage::on-object-created"
    );
    assert_eq!(
        function_id(EventKind::Deleted),
        "storage::on-object-deleted"
    );
    for (kind, id) in [
        (EventKind::Created, ON_OBJECT_CREATED),
        (EventKind::Deleted, ON_OBJECT_DELETED),
    ] {
        match refuse_invocation(kind) {
            Error::Remote { code, message, .. } => {
                assert_eq!(code, observe_codes::FORBIDDEN);
                assert_eq!(code, "FORBIDDEN");
                assert!(message.contains(id), "{message}");
            }
            other => panic!("expected FORBIDDEN, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn observer_receives_the_event_a_binding_receives() {
    for kind in KINDS {
        let registry = Arc::new(TriggerRegistry::new());
        // A binding of the trigger type on the same bucket.
        registry.register(
            "uploads".into(),
            kind,
            "b1".into(),
            "app::on-object".into(),
            5_000,
        );
        let observer = Recorder::new("o1");
        handler(kind, &registry)
            .add(observer.clone(), &json!({ "bucket": "uploads" }))
            .await
            .unwrap();
        assert_eq!(registry.subscribers_for("uploads", kind).len(), 1);
        assert_eq!(registry.observers_for("uploads", kind).len(), 1);
        // Bindings are untouched by observers.
        registry.unregister("b1");
        assert!(registry.subscribers_for("uploads", kind).is_empty());

        let normalized = event(kind, "uploads");
        let bound = event_payload(&normalized).unwrap();
        assert!(dispatcher(&registry).dispatch(normalized).await);
        assert_eq!(observer.seen(), vec![bound], "{kind:?}");
    }
}

#[tokio::test]
async fn observers_only_see_their_bucket_and_kind() {
    let registry = Arc::new(TriggerRegistry::new());
    let created = Recorder::new("created");
    let deleted = Recorder::new("deleted");
    handler(EventKind::Created, &registry)
        .add(created.clone(), &json!({ "bucket": "uploads" }))
        .await
        .unwrap();
    handler(EventKind::Deleted, &registry)
        .add(deleted.clone(), &json!({ "bucket": "uploads" }))
        .await
        .unwrap();

    let dispatcher = dispatcher(&registry);
    assert!(
        dispatcher
            .dispatch(event(EventKind::Created, "other"))
            .await
    );
    assert!(created.seen().is_empty() && deleted.seen().is_empty());

    assert!(
        dispatcher
            .dispatch(event(EventKind::Deleted, "uploads"))
            .await
    );
    assert!(created.seen().is_empty());
    assert_eq!(deleted.seen().len(), 1);
    assert_eq!(deleted.seen()[0]["key"], "a/b.txt");
}

#[tokio::test]
async fn observer_args_are_checked_like_a_binding_config() {
    let registry = Arc::new(TriggerRegistry::new());
    let observers = handler(EventKind::Created, &registry);
    let bindings = ObjectCreatedHandler {
        registry: registry.clone(),
        wired_buckets: wired(&["uploads"]),
        reconfigure_gate: Arc::new(RwLock::new(())),
    };
    for bad in [
        json!({}),
        json!({ "bucket": 7 }),
        json!({ "bucket": "unwired" }),
    ] {
        let rejection = observers.add(Recorder::new("o1"), &bad).await.unwrap_err();
        assert_eq!(rejection.code, observe_codes::INVALID_ARGS, "{bad}");
        let refused = bindings
            .register_trigger(TriggerConfig {
                id: "b1".into(),
                function_id: "app::on-object".into(),
                config: bad.clone(),
                metadata: None,
                namespace: None,
            })
            .await;
        assert!(refused.is_err(), "{bad}: the trigger type refuses it too");
    }
    assert!(registry
        .observers_for("unwired", EventKind::Created)
        .is_empty());
    assert!(registry
        .subscribers_for("unwired", EventKind::Created)
        .is_empty());
}

#[tokio::test]
async fn observer_results_follow_the_binding_ack_contract() {
    let normalized = || event(EventKind::Created, "uploads");
    let cases: [(Result<Value, Error>, bool); 5] = [
        (Ok(Value::Null), true),
        (Ok(json!({ "ack": false })), false),
        (
            Err(Error::Remote {
                code: observe_codes::FILTERS_NOT_MATCHED.into(),
                message: "declined".into(),
                stacktrace: None,
            }),
            true,
        ),
        (
            Err(Error::Remote {
                code: observe_codes::OBSERVER_NOT_ACTIVE.into(),
                message: "removed".into(),
                stacktrace: None,
            }),
            true,
        ),
        (Err(Error::Handler("target failed".into())), false),
    ];
    for (reply, acked) in cases {
        let registry = Arc::new(TriggerRegistry::new());
        handler(EventKind::Created, &registry)
            .add(
                Recorder::replying("o1", reply.clone()),
                &json!({ "bucket": "uploads" }),
            )
            .await
            .unwrap();
        assert_eq!(
            dispatcher(&registry).dispatch(normalized()).await,
            acked,
            "{reply:?}"
        );
    }
}

#[tokio::test]
async fn a_removed_observer_stops_receiving() {
    let registry = Arc::new(TriggerRegistry::new());
    let on_created = handler(EventKind::Created, &registry);
    let observer = Recorder::new("o1");
    on_created
        .add(observer.clone(), &json!({ "bucket": "uploads" }))
        .await
        .unwrap();
    // A replayed observer id replaces the entry instead of doubling it.
    on_created
        .add(observer.clone(), &json!({ "bucket": "uploads" }))
        .await
        .unwrap();
    assert_eq!(
        registry.observers_for("uploads", EventKind::Created).len(),
        1
    );

    on_created.remove("o1");
    assert!(registry
        .observers_for("uploads", EventKind::Created)
        .is_empty());
    assert!(
        dispatcher(&registry)
            .dispatch(event(EventKind::Created, "uploads"))
            .await
    );
    assert!(observer.seen().is_empty());
}
