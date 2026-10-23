//! Owned event feeds: the `devin::agent-event` trigger type carries the
//! translated AgentEvent frames and `devin::raw-event` the raw Devin CLI
//! stdout lines (`{ type: "stdout", line }`). The worker registers both at startup and delivers each
//! frame to the bindings for its session; nothing goes through a stream.
//!
//! Contract (shared by every agent worker's feed):
//! - binding config `{ session_id, metadata? }`: `session_id` is required,
//!   non-empty and at most 512 chars; `metadata` is an optional object that
//!   wins over the binding's own metadata; any other key, or a malformed
//!   value, rejects the binding. At most [`MAX_BINDINGS`] bindings per trigger
//!   type; re-registering an existing binding id replaces it.
//! - delivery payload `{ session_id, event_id, seq, epoch, source, event }`:
//!   `event` is the frame unchanged, `seq` is contiguous per (feed, session,
//!   epoch) starting at 0 with a separate counter per feed, `epoch` is a
//!   per-process uuid, `event_id` = `<session_id>-<epoch>-<seq:08>` and
//!   `source` = [`SOURCE`].
//! - every binding for the event's session gets one fire-and-forget (Void)
//!   trigger call, sequentially, with the binding's function id, namespace and
//!   metadata; failures are logged and swallowed so a turn never fails because
//!   a consumer is gone. No matching binding means no call at all.
//! - ephemeral: frames are not stored or replayed. Consumers order by
//!   `(epoch, seq)` and dedup by `event_id`; history lives in the session
//!   record and the run result.
//!
//! The binding table and counters live behind a `std::sync::Mutex`; matching
//! bindings are cloned out before any trigger call is awaited.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use serde_json::{json, Value};

/// Producer namespace stamped on every delivery as `source`.
pub const SOURCE: &str = "devin";
/// Trigger type carrying the translated AgentEvent frames.
pub const AGENT_EVENT_TRIGGER_TYPE: &str = "devin::agent-event";
/// Trigger type carrying the raw Devin CLI stdout lines, verbatim.
pub const RAW_EVENT_TRIGGER_TYPE: &str = "devin::raw-event";
/// Bindings accepted per trigger type; a new binding beyond it is rejected.
pub const MAX_BINDINGS: usize = 256;
/// Longest accepted `session_id`, in characters.
pub const MAX_SESSION_ID_CHARS: usize = 512;

const AGENT_EVENT_DESCRIPTION: &str = "Translated AgentEvent frames of one devin session. \
     Config: { session_id, metadata? }. Payload: { session_id, event_id, seq, epoch, source, \
     event }. Ephemeral (not stored, not replayed); order by (epoch, seq).";
const RAW_EVENT_DESCRIPTION: &str = "Raw Devin CLI stdout lines ({ type: stdout, line }, \
     verbatim) of one devin session. Config: { session_id, metadata? }. Payload: { session_id, \
     event_id, seq, epoch, source, event }. Ephemeral (not stored, not replayed); order by \
     (epoch, seq).";

/// A validated binding config.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedConfig {
    pub session_id: String,
    pub metadata: Option<Value>,
}

/// Validate a binding config; the error message rejects the binding.
pub fn validate_feed_config(type_id: &str, raw: &Value) -> Result<FeedConfig, String> {
    let obj = raw.as_object().ok_or_else(|| {
        format!("{type_id}: config must be an object {{ session_id, metadata? }}")
    })?;
    if let Some(key) = obj
        .keys()
        .find(|key| key.as_str() != "session_id" && key.as_str() != "metadata")
    {
        return Err(format!(
            "{type_id}: unknown config key `{key}` (allowed: session_id, metadata)"
        ));
    }
    let session_id = match obj.get("session_id") {
        Some(Value::String(s)) if s.is_empty() => {
            return Err(format!("{type_id}: session_id must not be empty"))
        }
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(format!("{type_id}: session_id must be a string")),
        None => return Err(format!("{type_id}: session_id is required")),
    };
    if session_id.chars().count() > MAX_SESSION_ID_CHARS {
        return Err(format!(
            "{type_id}: session_id is longer than {MAX_SESSION_ID_CHARS} characters"
        ));
    }
    let metadata = match obj.get("metadata") {
        None => None,
        Some(value @ Value::Object(_)) => Some(value.clone()),
        Some(_) => return Err(format!("{type_id}: metadata must be an object")),
    };
    Ok(FeedConfig {
        session_id,
        metadata,
    })
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    namespace: Option<String>,
    metadata: Option<Value>,
    config: FeedConfig,
}

/// One prepared trigger call: target, namespace, metadata and payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub function_id: String,
    pub namespace: Option<String>,
    pub metadata: Option<Value>,
    pub payload: Value,
}

impl Delivery {
    /// The fire-and-forget (Void) trigger request for this delivery.
    pub fn into_request(self) -> TriggerRequestWithMetadata {
        let mut request: TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id,
            payload: self.payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(namespace) = self.namespace {
            request = request.namespace(namespace);
        }
        if let Some(metadata) = self.metadata {
            request = request.metadata(metadata);
        }
        request
    }
}

#[derive(Default)]
struct FeedState {
    bindings: HashMap<String, Binding>,
    seq_by_session: HashMap<String, u64>,
}

/// One owned trigger type: its bindings and per-session sequence counters.
pub struct Feed {
    id: String,
    source: String,
    epoch: String,
    state: Mutex<FeedState>,
}

impl Feed {
    pub fn new(id: impl Into<String>, source: impl Into<String>, epoch: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            epoch: epoch.into(),
            state: Mutex::new(FeedState::default()),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Validate and store a binding (replacing one with the same id).
    pub fn bind(&self, binding: &TriggerConfig) -> Result<(), String> {
        let config = validate_feed_config(&self.id, &binding.config)?;
        let mut state = self.lock();
        if !state.bindings.contains_key(&binding.id) && state.bindings.len() >= MAX_BINDINGS {
            return Err(format!(
                "{}: binding limit ({MAX_BINDINGS}) reached",
                self.id
            ));
        }
        state.bindings.insert(
            binding.id.clone(),
            Binding {
                function_id: binding.function_id.clone(),
                namespace: binding.namespace.clone(),
                metadata: binding.metadata.clone(),
                config,
            },
        );
        Ok(())
    }

    /// Remove a binding; true when it existed.
    pub fn unbind(&self, binding_id: &str) -> bool {
        self.lock().bindings.remove(binding_id).is_some()
    }

    pub fn binding_count(&self) -> usize {
        self.lock().bindings.len()
    }

    /// Assign the next sequence number for `session_id` on this feed and
    /// build one delivery per matching binding (empty when none match). The
    /// lock is released before this returns.
    pub fn prepare(&self, session_id: &str, event: Value) -> Vec<Delivery> {
        let (seq, targets) = {
            let mut state = self.lock();
            let counter = state
                .seq_by_session
                .entry(session_id.to_string())
                .or_insert(0);
            let seq = *counter;
            *counter += 1;
            let targets: Vec<Binding> = state
                .bindings
                .values()
                .filter(|binding| binding.config.session_id == session_id)
                .cloned()
                .collect();
            (seq, targets)
        };
        if targets.is_empty() {
            return Vec::new();
        }
        let payload = json!({
            "session_id": session_id,
            "event_id": format!("{session_id}-{}-{seq:08}", self.epoch),
            "seq": seq,
            "epoch": self.epoch,
            "source": self.source,
            "event": event,
        });
        targets
            .into_iter()
            .map(|binding| Delivery {
                function_id: binding.function_id,
                namespace: binding.namespace,
                metadata: binding.config.metadata.or(binding.metadata),
                payload: payload.clone(),
            })
            .collect()
    }

    /// Deliver one frame through `send`, sequentially; failures are logged
    /// and swallowed. Returns how many deliveries succeeded.
    pub async fn emit_with<F, Fut>(&self, session_id: &str, event: Value, mut send: F) -> usize
    where
        F: FnMut(Delivery) -> Fut,
        Fut: Future<Output = Result<Value, Error>>,
    {
        let mut delivered = 0;
        for delivery in self.prepare(session_id, event) {
            let function_id = delivery.function_id.clone();
            match send(delivery).await {
                Ok(_) => delivered += 1,
                Err(e) => tracing::warn!(
                    trigger_type = %self.id,
                    function_id = %function_id,
                    session_id,
                    error = %e,
                    "event feed delivery failed"
                ),
            }
        }
        delivered
    }

    /// Deliver one frame to the matching bindings through the engine.
    pub async fn emit(&self, iii: &IIIClient, session_id: &str, event: Value) -> usize {
        self.emit_with(session_id, event, |delivery| {
            iii.trigger(delivery.into_request())
        })
        .await
    }
}

/// The engine-facing handler of one feed's trigger type.
pub struct FeedHandler {
    feed: Arc<Feed>,
}

impl FeedHandler {
    pub fn new(feed: Arc<Feed>) -> Self {
        Self { feed }
    }
}

#[async_trait]
impl TriggerHandler for FeedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.feed.bind(&config).map_err(|message| {
            tracing::warn!(trigger_type = %self.feed.id, binding = %config.id, %message, "binding rejected");
            Error::Handler(message)
        })
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.feed.unbind(&config.id);
        Ok(())
    }
}

/// The worker's two feeds; both share the process epoch, each keeps its own
/// counters.
pub struct Feeds {
    pub agent: Arc<Feed>,
    pub raw: Arc<Feed>,
}

impl Feeds {
    pub fn new(epoch: &str) -> Self {
        Self {
            agent: Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, epoch)),
            raw: Arc::new(Feed::new(RAW_EVENT_TRIGGER_TYPE, SOURCE, epoch)),
        }
    }
}

/// The process-wide feeds (epoch = a uuid minted once per process).
pub fn feeds() -> &'static Feeds {
    static FEEDS: OnceLock<Feeds> = OnceLock::new();
    FEEDS.get_or_init(|| Feeds::new(&uuid::Uuid::new_v4().to_string()))
}

/// Register both trigger types with the engine. Call once at startup.
pub fn register(iii: &IIIClient) {
    let feeds = feeds();
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        AGENT_EVENT_TRIGGER_TYPE,
        AGENT_EVENT_DESCRIPTION,
        FeedHandler::new(feeds.agent.clone()),
    ));
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        RAW_EVENT_TRIGGER_TYPE,
        RAW_EVENT_DESCRIPTION,
        FeedHandler::new(feeds.raw.clone()),
    ));
}

/// Deliver a translated AgentEvent frame on `devin::agent-event`.
pub async fn emit_agent_event(iii: &IIIClient, session_id: &str, event: Value) -> usize {
    feeds().agent.emit(iii, session_id, event).await
}

/// Deliver a raw CLI frame on `devin::raw-event`.
pub async fn emit_raw_event(iii: &IIIClient, session_id: &str, event: Value) -> usize {
    feeds().raw.emit(iii, session_id, event).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: &str = "epoch-test";

    fn binding(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    /// Emit through a capturing sender; returns (delivered count, captured).
    async fn capture(feed: &Feed, session_id: &str, event: Value) -> (usize, Vec<Delivery>) {
        let sent = Mutex::new(Vec::new());
        let delivered = feed
            .emit_with(session_id, event, |delivery| {
                sent.lock().unwrap().push(delivery);
                std::future::ready(Ok::<Value, Error>(Value::Null))
            })
            .await;
        (delivered, sent.into_inner().unwrap())
    }

    #[tokio::test]
    async fn delivers_to_bound_consumer_with_namespace_metadata_and_void_action() {
        let feeds = Feeds::new(EPOCH);
        let handler = FeedHandler::new(feeds.agent.clone());
        let mut with_config_meta = binding(
            "b1",
            "acp::__on_event::c1",
            json!({ "session_id": "s1", "metadata": { "from": "config" } }),
        );
        with_config_meta.namespace = Some("tenant-a".into());
        with_config_meta.metadata = Some(json!({ "from": "binding" }));
        handler.register_trigger(with_config_meta).await.unwrap();
        let mut with_binding_meta = binding("b2", "other::fn", json!({ "session_id": "s1" }));
        with_binding_meta.metadata = Some(json!({ "from": "binding" }));
        handler.register_trigger(with_binding_meta).await.unwrap();

        let event = json!({ "type": "message_complete", "message": { "role": "assistant" } });
        let (delivered, mut sent) = capture(&feeds.agent, "s1", event.clone()).await;
        assert_eq!(delivered, 2);
        sent.sort_by(|a, b| a.function_id.cmp(&b.function_id));

        let first = &sent[0];
        assert_eq!(first.function_id, "acp::__on_event::c1");
        assert_eq!(first.namespace.as_deref(), Some("tenant-a"));
        assert_eq!(first.metadata, Some(json!({ "from": "config" })));
        assert_eq!(
            first.payload,
            json!({
                "session_id": "s1",
                "event_id": format!("s1-{EPOCH}-00000000"),
                "seq": 0,
                "epoch": EPOCH,
                "source": SOURCE,
                "event": event,
            })
        );
        assert_eq!(sent[1].function_id, "other::fn");
        assert_eq!(sent[1].namespace, None);
        assert_eq!(sent[1].metadata, Some(json!({ "from": "binding" })));

        // The engine request is fire-and-forget and carries namespace + metadata.
        let request = format!("{:?}", first.clone().into_request());
        assert!(request.contains("action: Some(Void)"), "{request}");
        assert!(
            request.contains("namespace: Some(\"tenant-a\")"),
            "{request}"
        );
        assert!(
            request.contains("\"from\": String(\"config\")"),
            "{request}"
        );
        assert!(
            request.contains("function_id: \"acp::__on_event::c1\""),
            "{request}"
        );
    }

    #[tokio::test]
    async fn metadata_is_omitted_when_neither_config_nor_binding_has_it() {
        let feed = Feed::new(RAW_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        feed.bind(&binding("b", "f", json!({ "session_id": "s" })))
            .unwrap();
        let (_, sent) = capture(&feed, "s", json!({ "type": "turn.started" })).await;
        assert_eq!(sent[0].metadata, None);
        let request = format!("{:?}", sent[0].clone().into_request());
        assert!(request.contains("metadata: None"), "{request}");
    }

    #[tokio::test]
    async fn binding_for_one_session_never_sees_another() {
        let feed = Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        feed.bind(&binding("a", "fn::a", json!({ "session_id": "A" })))
            .unwrap();
        feed.bind(&binding("b", "fn::b", json!({ "session_id": "B" })))
            .unwrap();
        let (delivered, sent) = capture(&feed, "B", json!({ "type": "agent_end" })).await;
        assert_eq!(delivered, 1);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].function_id, "fn::b");
        assert_eq!(sent[0].payload["session_id"], "B");
        let (delivered, sent) = capture(&feed, "C", json!({ "type": "agent_end" })).await;
        assert_eq!(delivered, 0);
        assert!(sent.is_empty(), "no binding, no trigger call");
    }

    #[tokio::test]
    async fn no_delivery_after_unbind() {
        let feed = Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH));
        let handler = FeedHandler::new(feed.clone());
        let config = binding("b", "fn::b", json!({ "session_id": "s" }));
        handler.register_trigger(config.clone()).await.unwrap();
        assert_eq!(capture(&feed, "s", json!({ "type": "x" })).await.0, 1);
        handler.unregister_trigger(config).await.unwrap();
        assert_eq!(feed.binding_count(), 0);
        let (delivered, sent) = capture(&feed, "s", json!({ "type": "x" })).await;
        assert_eq!(delivered, 0);
        assert!(sent.is_empty());
    }

    #[tokio::test]
    async fn invalid_configs_are_rejected() {
        let feed = Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH));
        let handler = FeedHandler::new(feed.clone());
        let too_long = "x".repeat(MAX_SESSION_ID_CHARS + 1);
        for config in [
            json!({}),
            json!(null),
            json!("s1"),
            json!({ "session_id": "" }),
            json!({ "session_id": 7 }),
            json!({ "session_id": too_long }),
            json!({ "session_id": "s1", "group_id": "s1" }),
            json!({ "session_id": "s1", "metadata": "x" }),
            json!({ "session_id": "s1", "metadata": [1] }),
            json!({ "session_id": "s1", "metadata": null }),
        ] {
            let err = handler
                .register_trigger(binding("b", "f", config.clone()))
                .await
                .expect_err(&format!("{config} must be rejected"));
            assert!(matches!(err, Error::Handler(_)), "{err:?}");
        }
        assert_eq!(feed.binding_count(), 0);
        let max = "y".repeat(MAX_SESSION_ID_CHARS);
        assert!(validate_feed_config("t", &json!({ "session_id": max })).is_ok());
    }

    #[tokio::test]
    async fn binding_cap_rejects_new_ids_but_allows_replacing() {
        let feed = Arc::new(Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH));
        let handler = FeedHandler::new(feed.clone());
        for i in 0..MAX_BINDINGS {
            handler
                .register_trigger(binding(&format!("b{i}"), "f", json!({ "session_id": "s" })))
                .await
                .unwrap();
        }
        assert_eq!(feed.binding_count(), MAX_BINDINGS);
        let err = handler
            .register_trigger(binding("overflow", "f", json!({ "session_id": "s" })))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("binding limit (256)"), "{err}");
        // Re-registering an existing id replaces it, even at the cap.
        handler
            .register_trigger(binding("b0", "replaced", json!({ "session_id": "other" })))
            .await
            .unwrap();
        assert_eq!(feed.binding_count(), MAX_BINDINGS);
        let (_, sent) = capture(&feed, "other", json!({ "type": "x" })).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].function_id, "replaced");
    }

    #[tokio::test]
    async fn seq_is_contiguous_per_feed_and_session() {
        let feeds = Feeds::new(EPOCH);
        for feed in [&feeds.agent, &feeds.raw] {
            for session in ["s1", "s2"] {
                feed.bind(&binding(
                    &format!("{}-{session}", feed.id()),
                    "f",
                    json!({ "session_id": session }),
                ))
                .unwrap();
            }
        }
        let mut agent_s1 = Vec::new();
        let mut raw_s1 = Vec::new();
        for _ in 0..3 {
            raw_s1.extend(capture(&feeds.raw, "s1", json!({ "type": "raw" })).await.1);
            agent_s1.extend(capture(&feeds.agent, "s1", json!({ "type": "a" })).await.1);
        }
        raw_s1.extend(capture(&feeds.raw, "s1", json!({ "type": "raw" })).await.1);
        let agent_s2 = capture(&feeds.agent, "s2", json!({ "type": "a" })).await.1;

        let seqs = |sent: &[Delivery]| -> Vec<u64> {
            sent.iter()
                .map(|d| d.payload["seq"].as_u64().unwrap())
                .collect()
        };
        assert_eq!(seqs(&agent_s1), vec![0, 1, 2]);
        assert_eq!(seqs(&raw_s1), vec![0, 1, 2, 3]);
        assert_eq!(seqs(&agent_s2), vec![0]);
        assert_eq!(
            agent_s1[2].payload["event_id"],
            format!("s1-{EPOCH}-00000002")
        );
        assert_eq!(
            raw_s1[3].payload["event_id"],
            format!("s1-{EPOCH}-00000003")
        );
        assert_eq!(agent_s1[0].payload["epoch"], raw_s1[0].payload["epoch"]);
    }

    #[tokio::test]
    async fn seq_advances_without_bindings() {
        let feed = Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        assert!(feed.prepare("s", json!({ "type": "a" })).is_empty());
        feed.bind(&binding("b", "f", json!({ "session_id": "s" })))
            .unwrap();
        let sent = feed.prepare("s", json!({ "type": "a" }));
        assert_eq!(sent[0].payload["seq"], 1);
    }

    #[tokio::test]
    async fn failing_delivery_is_swallowed() {
        let feed = Feed::new(AGENT_EVENT_TRIGGER_TYPE, SOURCE, EPOCH);
        feed.bind(&binding("bad", "fn::gone", json!({ "session_id": "s" })))
            .unwrap();
        feed.bind(&binding("good", "fn::live", json!({ "session_id": "s" })))
            .unwrap();
        let attempted = Mutex::new(Vec::new());
        let delivered = feed
            .emit_with("s", json!({ "type": "agent_end" }), |delivery| {
                attempted.lock().unwrap().push(delivery.function_id.clone());
                std::future::ready(if delivery.function_id == "fn::gone" {
                    Err(Error::NotConnected)
                } else {
                    Ok(Value::Null)
                })
            })
            .await;
        assert_eq!(delivered, 1);
        assert_eq!(attempted.into_inner().unwrap().len(), 2);
        // The next frame still flows with the next seq.
        let (delivered, sent) = capture(&feed, "s", json!({ "type": "x" })).await;
        assert_eq!(delivered, 2);
        assert_eq!(sent[0].payload["seq"], 1);
    }

    #[test]
    fn process_feeds_use_the_owned_trigger_type_ids() {
        let feeds = feeds();
        assert_eq!(feeds.agent.id(), "devin::agent-event");
        assert_eq!(feeds.raw.id(), "devin::raw-event");
        assert_eq!(feeds.agent.epoch(), feeds.raw.epoch());
        assert!(uuid::Uuid::parse_str(feeds.agent.epoch()).is_ok());
    }
}
