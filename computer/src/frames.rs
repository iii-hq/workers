//! `computer::frame-changed`: the worker-owned trigger type behind the live
//! viewport. The newest frame of each session lives in exactly one in-memory
//! slot ([`crate::session::Session::latest_frame`]); this module tells
//! viewers that the slot changed, and they read it with `computer::frame`
//! (notify, then fetch). The notification carries no image: frames are
//! 60 KB-2 MB each, a payload copy per binding per frame would be the very
//! unbounded data path this replaces.
//!
//! Bounds, all enforced here:
//! - at most [`MAX_FRAME_BINDINGS`] live bindings; more are rejected;
//! - a required `session_id` filter, so a binding only hears one session;
//! - per binding, one [`DeliverySlot`]: at most ONE delivery in flight and
//!   ONE pending notification. A newer notification replaces the pending one
//!   (latest wins), so a slow or stuck viewer costs O(1) memory and one task,
//!   never a queue;
//! - delivery is a synchronous call with a timeout (not fire-and-forget), so a
//!   slow consumer keeps its slot busy and the provider coalesces instead of
//!   pushing into engine or socket buffers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FRAME_CHANGED: &str = "computer::frame-changed";
pub const FRAME_CHANGED_DESC: &str =
    "The newest screencast frame of one session changed (a new frame was stored, or the frame was \
     cleared because the screencast or session stopped). Config: { session_id } (required). \
     Payload: { session_id, epoch, frame_seq, change, width, height, mime?, bytes?, timestamp }; \
     no image: read it with computer::frame. Latest state only: intermediate notifications are \
     coalesced for slow consumers, nothing is stored or replayed.";
/// Upper bound on live `computer::frame-changed` bindings.
pub const MAX_FRAME_BINDINGS: usize = 64;
/// Per-delivery timeout; a consumer slower than this only loses intermediate
/// notifications, never blocks the pump.
pub const DELIVERY_TIMEOUT_MS: u64 = 5_000;

/// Binding filter for `computer::frame-changed`. `session_id` is required:
/// a viewer follows one desktop; unknown keys fail the registration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameBindingConfig {
    /// The computer session whose frames to follow.
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameChangeKind {
    /// A new frame was stored; read it with `computer::frame`.
    Updated,
    /// The stored frame was dropped (screencast stopped, capture failed, or
    /// the session ended). Nothing to read until the next `updated`.
    Cleared,
}

/// The notification. Small and image-free.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameChange {
    pub session_id: String,
    /// Identifies this incarnation of the session in this worker process
    /// (ms when it was created or restored). `frame_seq` restarts at 1 when
    /// `epoch` changes; compare `(epoch, frame_seq)` to order frames.
    pub epoch: i64,
    /// Sequence of the newest stored frame (monotonic within an epoch). For
    /// `cleared`, the sequence of the last frame that was stored.
    pub frame_seq: u64,
    pub change: FrameChangeKind,
    pub width: u32,
    pub height: u32,
    /// Image mime of the stored frame (`updated` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// Encoded image size in bytes (`updated` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub timestamp: i64,
}

/// Where a binding's notifications go, preserved from its registration.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameTarget {
    pub function_id: String,
    pub namespace: Option<String>,
    pub metadata: Option<Value>,
}

/// How one notification reaches its target. Production calls the bus;
/// tests record (and can be slow on purpose).
#[async_trait]
pub trait FrameSink: Send + Sync {
    async fn deliver(&self, target: &FrameTarget, payload: Value) -> Result<(), String>;
}

/// Bus delivery: a synchronous call with a timeout, preserving the binding's
/// namespace and metadata.
pub struct IiiFrameSink {
    iii: Arc<IIIClient>,
}

impl IiiFrameSink {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl FrameSink for IiiFrameSink {
    async fn deliver(&self, target: &FrameTarget, payload: Value) -> Result<(), String> {
        let request = TriggerRequest {
            function_id: target.function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(DELIVERY_TIMEOUT_MS),
        };
        let result = match (&target.namespace, &target.metadata) {
            (Some(ns), Some(meta)) => {
                self.iii
                    .trigger(request.namespace(ns.clone()).metadata(meta.clone()))
                    .await
            }
            (Some(ns), None) => self.iii.trigger(request.namespace(ns.clone())).await,
            (None, Some(meta)) => self.iii.trigger(request.metadata(meta.clone())).await,
            (None, None) => self.iii.trigger(request).await,
        };
        result.map(|_| ()).map_err(|e| e.to_string())
    }
}

#[derive(Default)]
struct SlotState {
    pending: Option<Value>,
    in_flight: bool,
    closed: bool,
}

/// One binding's coalescing mailbox: one pending notification, one delivery
/// in flight.
pub struct DeliverySlot {
    target: FrameTarget,
    state: Mutex<SlotState>,
    delivered: AtomicU64,
    coalesced: AtomicU64,
}

impl DeliverySlot {
    fn new(target: FrameTarget) -> Arc<Self> {
        Arc::new(Self {
            target,
            state: Mutex::new(SlotState::default()),
            delivered: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Offer a notification. Replaces any pending one; starts the single
    /// drain task only when none is running.
    fn offer(self: &Arc<Self>, payload: Value, sink: &Arc<dyn FrameSink>) {
        let start = {
            let mut st = self.lock();
            if st.closed {
                return;
            }
            if st.pending.replace(payload).is_some() {
                self.coalesced.fetch_add(1, Ordering::Relaxed);
            }
            !std::mem::replace(&mut st.in_flight, true)
        };
        if start {
            let slot = self.clone();
            let sink = sink.clone();
            tokio::spawn(async move { slot.drain(sink).await });
        }
    }

    async fn drain(self: Arc<Self>, sink: Arc<dyn FrameSink>) {
        loop {
            let next = {
                let mut st = self.lock();
                match (st.closed, st.pending.take()) {
                    (false, Some(payload)) => payload,
                    _ => {
                        st.pending = None;
                        st.in_flight = false;
                        return;
                    }
                }
            };
            match sink.deliver(&self.target, next).await {
                Ok(()) => {
                    self.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::debug!(function_id = %self.target.function_id, error = %e, "frame-changed delivery failed");
                }
            }
        }
    }

    fn close(&self) {
        let mut st = self.lock();
        st.closed = true;
        st.pending = None;
    }

    /// Notifications waiting in this slot: 0 or 1, by construction.
    pub fn pending_len(&self) -> usize {
        usize::from(self.lock().pending.is_some())
    }

    pub fn in_flight(&self) -> bool {
        self.lock().in_flight
    }

    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::Relaxed)
    }

    pub fn coalesced(&self) -> u64 {
        self.coalesced.load(Ordering::Relaxed)
    }
}

struct FrameBinding {
    session_id: String,
    slot: Arc<DeliverySlot>,
}

/// Registry of `computer::frame-changed` bindings plus their delivery slots.
pub struct FrameNotifier {
    bindings: Mutex<HashMap<String, FrameBinding>>,
    sink: Arc<dyn FrameSink>,
}

impl FrameNotifier {
    pub fn new(sink: Arc<dyn FrameSink>) -> Arc<Self> {
        Arc::new(Self {
            bindings: Mutex::new(HashMap::new()),
            sink,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, FrameBinding>> {
        self.bindings.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Validate and store a binding. Rejects a missing/empty `session_id`,
    /// unknown keys, and anything past [`MAX_FRAME_BINDINGS`].
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter: FrameBindingConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| format!("invalid {FRAME_CHANGED} config: {e}"))?;
        let session_id = filter.session_id.trim().to_string();
        if session_id.is_empty() {
            return Err(format!(
                "invalid {FRAME_CHANGED} config: session_id must not be empty"
            ));
        }
        let mut map = self.lock();
        if !map.contains_key(&config.id) && map.len() >= MAX_FRAME_BINDINGS {
            return Err(format!(
                "{FRAME_CHANGED}: binding limit ({MAX_FRAME_BINDINGS}) reached"
            ));
        }
        let slot = DeliverySlot::new(FrameTarget {
            function_id: config.function_id,
            namespace: config.namespace,
            metadata: config.metadata,
        });
        if let Some(old) = map.insert(config.id, FrameBinding { session_id, slot }) {
            old.slot.close();
        }
        Ok(())
    }

    pub fn remove(&self, id: &str) {
        if let Some(binding) = self.lock().remove(id) {
            binding.slot.close();
        }
    }

    pub fn binding_count(&self) -> usize {
        self.lock().len()
    }

    /// The slot of one binding (tests and diagnostics).
    pub fn slot(&self, id: &str) -> Option<Arc<DeliverySlot>> {
        self.lock().get(id).map(|b| b.slot.clone())
    }

    /// Offer `change` to every binding for its session. Never blocks on a
    /// consumer: the slot coalesces and its single drain task delivers.
    pub fn notify(&self, change: &FrameChange) {
        let slots: Vec<Arc<DeliverySlot>> = self
            .lock()
            .values()
            .filter(|b| b.session_id == change.session_id)
            .map(|b| b.slot.clone())
            .collect();
        if slots.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(change) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "frame-changed payload failed to serialize");
                return;
            }
        };
        for slot in slots {
            slot.offer(payload.clone(), &self.sink);
        }
    }
}

struct FrameTriggerHandler {
    notifier: Arc<FrameNotifier>,
}

#[async_trait]
impl TriggerHandler for FrameTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.notifier.add(config).map_err(Error::Handler)?;
        tracing::info!(id = %id, function_id = %function_id, "frame-changed binding registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.notifier.remove(&config.id);
        tracing::info!(id = %config.id, "frame-changed binding unregistered");
        Ok(())
    }
}

/// Register `computer::frame-changed` with the engine and return the notifier
/// the screencast pump feeds.
pub fn register_frame_trigger_type(iii: &Arc<IIIClient>) -> Arc<FrameNotifier> {
    let notifier = FrameNotifier::new(Arc::new(IiiFrameSink::new(iii.clone())));
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            FRAME_CHANGED,
            FRAME_CHANGED_DESC,
            FrameTriggerHandler {
                notifier: notifier.clone(),
            },
        )
        .trigger_request_format::<FrameBindingConfig>()
        .call_request_format::<FrameChange>(),
    );
    tracing::info!(trigger_type = FRAME_CHANGED, "registered trigger type");
    notifier
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    /// Records deliveries; each delivery takes `delay` (a slow consumer).
    pub(crate) struct Recorder {
        pub seen: Mutex<Vec<(FrameTarget, Value)>>,
        pub delay: Duration,
    }

    #[async_trait]
    impl FrameSink for Recorder {
        async fn deliver(&self, target: &FrameTarget, payload: Value) -> Result<(), String> {
            tokio::time::sleep(self.delay).await;
            self.seen.lock().unwrap().push((target.clone(), payload));
            Ok(())
        }
    }

    fn recorder(delay_ms: u64) -> Arc<Recorder> {
        Arc::new(Recorder {
            seen: Mutex::new(Vec::new()),
            delay: Duration::from_millis(delay_ms),
        })
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: format!("viewer::{id}"),
            config,
            metadata: Some(json!({ "viewer": id })),
            namespace: Some("tab-ns".to_string()),
        }
    }

    fn change(session: &str, seq: u64) -> FrameChange {
        FrameChange {
            session_id: session.to_string(),
            epoch: 7,
            frame_seq: seq,
            change: FrameChangeKind::Updated,
            width: 1280,
            height: 800,
            mime: Some("image/jpeg".to_string()),
            bytes: Some(100_000),
            timestamp: 0,
        }
    }

    async fn settle(notifier: &FrameNotifier, ids: &[&str]) {
        for _ in 0..500 {
            if ids
                .iter()
                .all(|id| notifier.slot(id).map(|s| !s.in_flight()).unwrap_or(true))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("slots did not settle");
    }

    #[test]
    fn config_requires_a_session_id_and_rejects_unknown_keys() {
        let n = FrameNotifier::new(recorder(0));
        assert!(n.add(binding("a", Value::Null)).is_err());
        assert!(n.add(binding("a", json!({}))).is_err());
        assert!(n.add(binding("a", json!({ "session_id": "  " }))).is_err());
        let err = n
            .add(binding(
                "a",
                json!({ "session_id": "c1", "stream_name": "x" }),
            ))
            .unwrap_err();
        assert!(err.contains(FRAME_CHANGED), "{err}");
        n.add(binding("a", json!({ "session_id": "c1" }))).unwrap();
        assert_eq!(n.binding_count(), 1);
    }

    #[test]
    fn binding_count_is_capped() {
        let n = FrameNotifier::new(recorder(0));
        for i in 0..MAX_FRAME_BINDINGS {
            n.add(binding(&format!("b{i}"), json!({ "session_id": "c1" })))
                .unwrap();
        }
        let err = n
            .add(binding("one-too-many", json!({ "session_id": "c1" })))
            .unwrap_err();
        assert!(err.contains("binding limit"), "{err}");
        // Re-registering an existing id is an update, not a new binding.
        n.add(binding("b0", json!({ "session_id": "c2" }))).unwrap();
        assert_eq!(n.binding_count(), MAX_FRAME_BINDINGS);
    }

    #[tokio::test]
    async fn notifications_are_filtered_by_session_and_keep_namespace_and_metadata() {
        let rec = recorder(0);
        let n = FrameNotifier::new(rec.clone());
        n.add(binding("one", json!({ "session_id": "c1" })))
            .unwrap();
        n.add(binding("two", json!({ "session_id": "c2" })))
            .unwrap();
        n.notify(&change("c1", 1));
        settle(&n, &["one", "two"]).await;
        let seen = rec.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let (target, payload) = &seen[0];
        assert_eq!(target.function_id, "viewer::one");
        assert_eq!(target.namespace.as_deref(), Some("tab-ns"));
        assert_eq!(target.metadata, Some(json!({ "viewer": "one" })));
        assert_eq!(payload["session_id"], "c1");
        assert_eq!(payload["frame_seq"], 1);
        assert_eq!(payload["change"], "updated");
        // Image-free: the payload stays small regardless of frame size.
        assert!(payload.get("data").is_none() && payload.get("frame").is_none());
        assert!(payload.to_string().len() < 512);
    }

    #[tokio::test]
    async fn slow_consumer_is_coalesced_to_the_latest_with_bounded_memory() {
        // Each delivery takes 50ms; 1000 frames arrive back to back.
        let rec = recorder(50);
        let n = FrameNotifier::new(rec.clone());
        n.add(binding("slow", json!({ "session_id": "c1" })))
            .unwrap();
        let slot = n.slot("slow").unwrap();
        let mut max_pending = 0;
        for seq in 1..=1000 {
            n.notify(&change("c1", seq));
            max_pending = max_pending.max(slot.pending_len());
        }
        assert!(max_pending <= 1, "pending grew to {max_pending}");
        settle(&n, &["slow"]).await;
        let seqs: Vec<u64> = rec
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p["frame_seq"].as_u64().unwrap())
            .collect();
        // The first frame went out immediately, everything after it was
        // coalesced into one pending slot: the consumer saw the newest.
        assert!(seqs.len() <= 3, "delivered {seqs:?}");
        assert_eq!(*seqs.last().unwrap(), 1000);
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
        assert_eq!(slot.delivered() as usize, seqs.len());
        assert!(slot.coalesced() >= 997, "coalesced {}", slot.coalesced());
        assert_eq!(slot.pending_len(), 0);
    }

    #[tokio::test]
    async fn unregister_drops_the_pending_notification() {
        let rec = recorder(30);
        let n = FrameNotifier::new(rec.clone());
        n.add(binding("gone", json!({ "session_id": "c1" })))
            .unwrap();
        let slot = n.slot("gone").unwrap();
        n.notify(&change("c1", 1));
        tokio::time::sleep(Duration::from_millis(5)).await; // 1 is now in flight
        n.notify(&change("c1", 2)); // pending
        n.remove("gone");
        assert_eq!(n.binding_count(), 0);
        assert_eq!(slot.pending_len(), 0);
        n.notify(&change("c1", 3)); // nobody bound
        tokio::time::sleep(Duration::from_millis(80)).await;
        let seqs: Vec<u64> = rec
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p["frame_seq"].as_u64().unwrap())
            .collect();
        assert_eq!(seqs, vec![1]);
        assert!(!slot.in_flight());
    }
}
