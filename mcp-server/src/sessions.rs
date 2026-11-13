//! MCP sessions and their observers.
//!
//! Streamable HTTP gives each client an `Mcp-Session-Id` at `initialize`.
//! A session owns the observers it creates. Each one delivers its
//! observations one of three ways ([`Delivery`]):
//!
//! - **poll** (`iii_observe`): the engine sends them to the shared sink
//!   (`mcp-server::observed`), which buffers them in the session until the client
//!   reads them with `iii_observations_poll`;
//! - **sse** (`iii_observations_sse`): through the sink, then written to the
//!   session's `GET` SSE stream with an event id. They stay in the buffer
//!   after that: a write the socket took is no proof the client got it, so a
//!   reconnect with `Last-Event-ID` replays everything after that id;
//! - **target** (`iii_observe` with `target_function_id`): the engine runs
//!   that function for each one; nothing reaches the session.
//!
//! The buffer is per session (oldest dropped first). A session has at most
//! `max_observers` observers, and the store at most `max_sessions` sessions.
//! Sessions idle longer than the TTL are ended, disposing their observers:
//! HTTP clients do not always send `DELETE` when they go away.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

pub use crate::bus::BusError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// An observer the engine is setting up (an iii-sdk `ObserverHandle`).
pub trait ObserverLike: Send + Sync {
    fn id(&self) -> String;
    fn state(&self) -> String;
    fn ready(&self) -> BoxFuture<'_, Result<(), BusError>>;
    fn dispose(&self) -> BoxFuture<'_, ()>;
}

/// Creates observers on the engine.
pub trait Observe: Send + Sync {
    fn observe(&self, req: &ObserveRequest, target: &str) -> Box<dyn ObserverLike>;
}

/// The source half of the observe tools' arguments.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ObserveRequest {
    #[serde(default)]
    pub function_id: String,
    #[serde(default)]
    pub filters: Option<Value>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub capture: Option<Vec<String>>,
    #[serde(default)]
    pub namespace: Option<String>,
    /// The observer ends after this many observations.
    #[serde(default)]
    pub max_observations: Option<u64>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

/// Where an observer's observations go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Buffered in the session for `iii_observations_poll`.
    Poll,
    /// Pushed on the session's SSE stream.
    Sse,
    /// The engine runs this function with each observation.
    Target(String),
}

impl Delivery {
    fn name(&self) -> &'static str {
        match self {
            Delivery::Poll => "poll",
            Delivery::Sse => "sse",
            Delivery::Target(_) => "target",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SessionLimits {
    /// Live sessions; `initialize` is refused beyond it.
    pub max_sessions: usize,
    /// Observers per session.
    pub max_observers: usize,
    /// Observations held per session; the oldest are dropped beyond it.
    pub max_buffered: usize,
    /// A session nobody used for this long is ended.
    pub idle_ttl: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_sessions: 16,
            max_observers: 100,
            max_buffered: 500,
            idle_ttl: Duration::from_secs(30 * 60),
        }
    }
}

/// What the session's SSE stream writes.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// One observation; `id` is its SSE event id, unique within the session.
    Observation { id: u64, observation: Value },
    /// Observations after `after`, up to `through`, were dropped by the
    /// buffer cap before this stream could send them.
    Gap { after: u64, through: u64 },
}

/// How long an observe tool waits for the engine to activate an observer.
/// An observer of a function nobody hosts yet stays `pending`, which is valid.
const READY_WAIT: Duration = Duration::from_secs(5);

struct Entry {
    function_id: String,
    filters: Option<Value>,
    delivery: Delivery,
    handle: Arc<dyn ObserverLike>,
    received: u64,
    dropped: u64,
}

struct Session {
    last_seen: Instant,
    order: Vec<String>,
    observers: HashMap<String, Entry>,
    /// Poll observations until polled, and SSE ones until the cap drops
    /// them (for replay): (seq, observer id, observation).
    buffer: VecDeque<(u64, String, Value)>,
    /// Dropped since the last poll.
    dropped: u64,
    stream: Option<mpsc::UnboundedSender<StreamItem>>,
    /// The highest SSE event id a stream wrote.
    sent_through: u64,
    /// The highest SSE event id the cap dropped.
    sse_dropped_through: u64,
}

impl Session {
    fn buffered_for(&self, observer_id: &str) -> usize {
        self.buffer
            .iter()
            .filter(|(_, id, _)| id == observer_id)
            .count()
    }

    fn is_sse(&self, observer_id: &str) -> bool {
        self.observers
            .get(observer_id)
            .is_some_and(|e| e.delivery == Delivery::Sse)
    }

    /// Push to the open stream, if any; a closed one is forgotten.
    fn push(&mut self, item: StreamItem) {
        if let Some(stream) = &self.stream {
            if stream.send(item).is_err() {
                self.stream = None;
            }
        }
    }
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    /// observer id -> session id, for routing deliveries.
    owner: HashMap<String, String>,
    seq: u64,
}

pub struct Sessions {
    limits: SessionLimits,
    inner: Mutex<Inner>,
}

impl Sessions {
    pub fn new(limits: SessionLimits) -> Self {
        Self {
            limits,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Open a session, unless `max_sessions` are already open.
    pub fn create(&self) -> Result<String, String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.sessions.len() >= self.limits.max_sessions {
            return Err(format!(
                "this MCP server already has {} sessions (max_sessions); end one with DELETE or wait for an idle one to end",
                self.limits.max_sessions
            ));
        }
        let id = Uuid::new_v4().to_string();
        inner.sessions.insert(
            id.clone(),
            Session {
                last_seen: Instant::now(),
                order: vec![],
                observers: HashMap::new(),
                buffer: VecDeque::new(),
                dropped: 0,
                stream: None,
                sent_through: 0,
                sse_dropped_through: 0,
            },
        );
        Ok(id)
    }

    /// Mark a session used; `false` when it does not exist.
    pub fn touch(&self, session_id: &str) -> bool {
        match self.inner.lock().unwrap().sessions.get_mut(session_id) {
            Some(s) => {
                s.last_seen = Instant::now();
                true
            }
            None => false,
        }
    }

    /// Create an observer for the session. Poll and SSE observers target
    /// `sink`; a target observer targets its function.
    pub async fn observe<O: Observe + ?Sized>(
        &self,
        observe: &O,
        session_id: &str,
        req: ObserveRequest,
        delivery: Delivery,
        sink: &str,
    ) -> Result<Value, String> {
        let function_id = req.function_id.trim().to_string();
        if function_id.is_empty() {
            return Err("function_id is required".into());
        }
        let delivery = match delivery {
            Delivery::Target(t) if t.trim().is_empty() => {
                return Err("target_function_id must name a function".into())
            }
            Delivery::Target(t) => Delivery::Target(t.trim().to_string()),
            other => other,
        };
        {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(session_id) else {
                return Err(format!("unknown session: {session_id}"));
            };
            session.last_seen = Instant::now();
            if session.observers.len() >= self.limits.max_observers {
                return Err(format!(
                    "this session already has {} observers (max_observers_per_session); remove one with iii_unobserve",
                    self.limits.max_observers
                ));
            }
        }
        let target = match &delivery {
            Delivery::Target(t) => t.as_str(),
            _ => sink,
        };
        let handle: Arc<dyn ObserverLike> = Arc::from(observe.observe(
            &ObserveRequest {
                function_id: function_id.clone(),
                ..req.clone()
            },
            target,
        ));
        let observer_id = handle.id();
        let recorded = {
            // Recorded before activation, so an early delivery is routed.
            let mut inner = self.inner.lock().unwrap();
            match inner.sessions.get_mut(session_id) {
                None => false,
                Some(session) => {
                    session.order.push(observer_id.clone());
                    session.observers.insert(
                        observer_id.clone(),
                        Entry {
                            function_id: function_id.clone(),
                            filters: req.filters.clone(),
                            delivery: delivery.clone(),
                            handle: handle.clone(),
                            received: 0,
                            dropped: 0,
                        },
                    );
                    inner
                        .owner
                        .insert(observer_id.clone(), session_id.to_string());
                    true
                }
            }
        };
        if !recorded {
            // The session ended while the engine was asked.
            handle.dispose().await;
            return Err(format!("unknown session: {session_id}"));
        }
        if let Ok(Err(e)) = tokio::time::timeout(READY_WAIT, handle.ready()).await {
            self.forget(session_id, &observer_id);
            handle.dispose().await;
            return Err(format!("observe refused: {}: {}", e.code, e.message));
        }
        let mut out = json!({
            "observer_id": observer_id,
            "function_id": function_id,
            "state": handle.state(),
            "delivery": delivery.name(),
        });
        if let Delivery::Target(t) = &delivery {
            out["target_function_id"] = json!(t);
        }
        Ok(out)
    }

    /// Remove an observer from its session; returns its entry and how many
    /// of its observations were discarded.
    fn forget(&self, session_id: &str, observer_id: &str) -> Option<(Entry, usize)> {
        let mut inner = self.inner.lock().unwrap();
        let (entry, discarded) = inner.sessions.get_mut(session_id).and_then(|s| {
            s.order.retain(|o| o != observer_id);
            let entry = s.observers.remove(observer_id)?;
            let before = s.buffer.len();
            s.buffer.retain(|(_, id, _)| id != observer_id);
            Some((entry, before - s.buffer.len()))
        })?;
        inner.owner.remove(observer_id);
        Some((entry, discarded))
    }

    /// Route one observation from the sink to its session; `false` when it
    /// belongs to no live poll or SSE observer (dropped, never resurrected).
    pub fn deliver(&self, observation: Value) -> bool {
        let Some(observer_id) = observation
            .get("observer_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return false;
        };
        let mut inner = self.inner.lock().unwrap();
        let Some(session_id) = inner.owner.get(&observer_id).cloned() else {
            return false;
        };
        inner.seq += 1;
        let seq = inner.seq;
        let max = self.limits.max_buffered;
        let Some(session) = inner.sessions.get_mut(&session_id) else {
            return false;
        };
        let Some(entry) = session.observers.get_mut(&observer_id) else {
            return false;
        };
        let sse = match entry.delivery {
            Delivery::Target(_) => return false,
            Delivery::Sse => true,
            Delivery::Poll => false,
        };
        entry.received += 1;
        if sse {
            session.push(StreamItem::Observation {
                id: seq,
                observation: observation.clone(),
            });
        }
        session.buffer.push_back((seq, observer_id, observation));
        while session.buffer.len() > max {
            if let Some((dropped_seq, id, _)) = session.buffer.pop_front() {
                session.dropped += 1;
                if session.is_sse(&id) {
                    session.sse_dropped_through = session.sse_dropped_through.max(dropped_seq);
                }
                if let Some(e) = session.observers.get_mut(&id) {
                    e.dropped += 1;
                }
            }
        }
        true
    }

    /// Read and remove poll observations, oldest first: one observer's, or
    /// all of the session's poll observers'.
    pub fn poll(
        &self,
        session_id: &str,
        observer_id: Option<&str>,
        max: usize,
    ) -> Result<Value, String> {
        let max = if max == 0 { 50 } else { max };
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        session.last_seen = Instant::now();
        if let Some(id) = observer_id {
            match session.observers.get(id) {
                None => return Err(format!("unknown observer: {id}")),
                Some(e) if e.delivery != Delivery::Poll => {
                    return Err(format!(
                        "observer {id} does not buffer for polling: it was created with {}",
                        if e.delivery == Delivery::Sse {
                            "iii_observations_sse (its observations arrive on the SSE stream)"
                        } else {
                            "a target_function_id (the engine runs that function instead)"
                        }
                    ))
                }
                Some(_) => {}
            }
        }
        let wanted = |id: &str| match observer_id {
            Some(o) => o == id,
            None => true,
        };
        let mut observations = Vec::new();
        let mut kept = VecDeque::with_capacity(session.buffer.len());
        let mut remaining = 0;
        let sse: Vec<String> = session
            .observers
            .iter()
            .filter(|(_, e)| e.delivery == Delivery::Sse)
            .map(|(id, _)| id.clone())
            .collect();
        for item in session.buffer.drain(..) {
            let is_poll = !sse.contains(&item.1);
            if is_poll && wanted(&item.1) {
                if observations.len() < max {
                    observations.push(item.2);
                    continue;
                }
                remaining += 1;
            }
            kept.push_back(item);
        }
        session.buffer = kept;
        let dropped = std::mem::take(&mut session.dropped);
        Ok(json!({ "observations": observations, "remaining": remaining, "dropped": dropped }))
    }

    /// Open the session's SSE stream, replacing any open one. The backlog is
    /// what the stream sends first: the SSE observations after
    /// `last_event_id` (a reconnect's `Last-Event-ID`), or, without one,
    /// those no stream wrote yet, led by a [`StreamItem::Gap`] when the cap
    /// dropped some of them. Live observations follow on the receiver.
    #[allow(clippy::type_complexity)]
    pub fn attach_stream(
        &self,
        session_id: &str,
        last_event_id: Option<u64>,
    ) -> Result<(Vec<StreamItem>, mpsc::UnboundedReceiver<StreamItem>), String> {
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        session.last_seen = Instant::now();
        let after = last_event_id.unwrap_or(session.sent_through);
        let mut backlog = Vec::new();
        if session.sse_dropped_through > after {
            backlog.push(StreamItem::Gap {
                after,
                through: session.sse_dropped_through,
            });
        }
        for (seq, id, observation) in &session.buffer {
            if *seq > after && session.is_sse(id) {
                backlog.push(StreamItem::Observation {
                    id: *seq,
                    observation: observation.clone(),
                });
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        session.stream = Some(tx);
        Ok((backlog, rx))
    }

    /// A stream wrote the observation with event id `id`.
    pub fn mark_sent(&self, session_id: &str, id: u64) {
        if let Some(session) = self.inner.lock().unwrap().sessions.get_mut(session_id) {
            session.sent_through = session.sent_through.max(id);
        }
    }

    pub fn list(&self, session_id: &str) -> Result<Vec<Value>, String> {
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        session.last_seen = Instant::now();
        Ok(session
            .order
            .iter()
            .filter_map(|id| session.observers.get(id).map(|e| (id, e)))
            .map(|(id, e)| {
                let mut v = json!({
                    "observer_id": id,
                    "function_id": e.function_id,
                    "delivery": e.delivery.name(),
                    "state": e.handle.state(),
                    "buffered": session.buffered_for(id),
                    "received": e.received,
                    "dropped": e.dropped,
                });
                if let Delivery::Target(t) = &e.delivery {
                    v["target_function_id"] = json!(t);
                }
                if let Some(f) = &e.filters {
                    v["filters"] = f.clone();
                }
                v
            })
            .collect())
    }

    pub async fn unobserve(&self, session_id: &str, observer_id: &str) -> Result<Value, String> {
        if !self.touch(session_id) {
            return Err(format!("unknown session: {session_id}"));
        }
        let (entry, discarded) = self
            .forget(session_id, observer_id)
            .ok_or_else(|| format!("unknown observer: {observer_id}"))?;
        entry.handle.dispose().await;
        Ok(json!({ "observer_id": observer_id, "removed": true, "discarded": discarded }))
    }

    /// End a session and dispose its observers; `false` when unknown.
    pub async fn end(&self, session_id: &str) -> bool {
        let handles: Vec<Arc<dyn ObserverLike>> = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.remove(session_id) else {
                return false;
            };
            for id in session.observers.keys() {
                inner.owner.remove(id);
            }
            session.observers.into_values().map(|e| e.handle).collect()
        };
        for h in handles {
            h.dispose().await;
        }
        true
    }

    /// End every session idle longer than the TTL; returns how many.
    pub async fn end_idle(&self) -> usize {
        let expired: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner
                .sessions
                .iter()
                .filter(|(_, s)| s.last_seen.elapsed() > self.limits.idle_ttl)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut n = 0;
        for id in expired {
            if self.end(&id).await {
                n += 1;
            }
        }
        n
    }

    pub async fn close_all(&self) {
        let ids: Vec<String> = self
            .inner
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.end(&id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// An observer the engine accepted (or refused) at once.
    struct FakeObserver {
        id: String,
        refused: Option<BusError>,
        disposed: Arc<AtomicUsize>,
    }

    impl ObserverLike for FakeObserver {
        fn id(&self) -> String {
            self.id.clone()
        }
        fn state(&self) -> String {
            if self.refused.is_some() {
                "error".into()
            } else {
                "active".into()
            }
        }
        fn ready(&self) -> BoxFuture<'_, Result<(), BusError>> {
            let r = self.refused.clone();
            Box::pin(async move { r.map_or(Ok(()), Err) })
        }
        fn dispose(&self) -> BoxFuture<'_, ()> {
            self.disposed.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    /// Creates observers `obs-1`, `obs-2`, ... and records what was asked.
    #[derive(Default)]
    struct FakeObserve {
        next: AtomicUsize,
        refuse: Option<BusError>,
        disposed: Arc<AtomicUsize>,
        asked: std::sync::Mutex<Vec<(ObserveRequest, String)>>,
    }

    impl Observe for FakeObserve {
        fn observe(&self, req: &ObserveRequest, target: &str) -> Box<dyn ObserverLike> {
            self.asked
                .lock()
                .unwrap()
                .push((req.clone(), target.to_string()));
            let n = self.next.fetch_add(1, Ordering::SeqCst) + 1;
            Box::new(FakeObserver {
                id: format!("obs-{n}"),
                refused: self.refuse.clone(),
                disposed: self.disposed.clone(),
            })
        }
    }

    const SINK: &str = "mcp-server::observed";

    fn limits(max_sessions: usize, max_observers: usize, max_buffered: usize) -> SessionLimits {
        SessionLimits {
            max_sessions,
            max_observers,
            max_buffered,
            idle_ttl: Duration::from_secs(60),
        }
    }

    fn store() -> Sessions {
        Sessions::new(limits(16, 100, 500))
    }

    fn observation(observer_id: &str, input: Value) -> Value {
        json!({ "observer_id": observer_id, "function_id": "state::set", "status": "success", "input": input })
    }

    fn req(function_id: &str) -> ObserveRequest {
        serde_json::from_value(json!({ "function_id": function_id })).unwrap()
    }

    fn inputs(v: &Value) -> Vec<Value> {
        v["observations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["input"].clone())
            .collect()
    }

    #[test]
    fn the_defaults_are_16_sessions_100_observers_and_500_buffered() {
        let d = SessionLimits::default();
        assert_eq!(
            (d.max_sessions, d.max_observers, d.max_buffered),
            (16, 100, 500)
        );
    }

    #[test]
    fn sessions_are_created_with_unique_ids_and_known_afterwards() {
        let s = store();
        let a = s.create().unwrap();
        let b = s.create().unwrap();
        assert_ne!(a, b);
        assert!(s.touch(&a));
        assert!(!s.touch("nope"));
    }

    #[tokio::test]
    async fn there_are_at_most_max_sessions_and_ending_one_frees_a_slot() {
        let s = Sessions::new(limits(2, 100, 500));
        let a = s.create().unwrap();
        s.create().unwrap();
        let err = s.create().unwrap_err();
        assert!(err.contains("2") && err.contains("max_sessions"), "{err}");
        assert!(s.end(&a).await);
        assert!(s.create().is_ok());
    }

    #[test]
    fn observe_request_parses_the_tool_arguments() {
        let r: ObserveRequest = serde_json::from_value(json!({
            "function_id": "orders::create",
            "filters": { "input.amount": { "$gte": 500 } },
            "args": { "table": "orders" },
            "capture": ["input", "error"],
            "namespace": "default",
            "max_observations": 3,
            "expires_in_ms": 60000
        }))
        .unwrap();
        assert_eq!(r.function_id, "orders::create");
        assert_eq!(
            r.capture.as_deref(),
            Some(&["input".to_string(), "error".to_string()][..])
        );
        assert_eq!(r.max_observations, Some(3));
    }

    #[tokio::test]
    async fn a_poll_observer_targets_the_sink() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        let r = s
            .observe(&o, &sid, req("state::set"), Delivery::Poll, SINK)
            .await
            .unwrap();
        assert_eq!(
            r,
            json!({ "observer_id": "obs-1", "function_id": "state::set", "state": "active", "delivery": "poll" })
        );
        assert_eq!(o.asked.lock().unwrap()[0].1, SINK);
    }

    #[tokio::test]
    async fn an_sse_observer_targets_the_sink_too() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        let r = s
            .observe(&o, &sid, req("state::set"), Delivery::Sse, SINK)
            .await
            .unwrap();
        assert_eq!(r["delivery"], "sse");
        assert_eq!(o.asked.lock().unwrap()[0].1, SINK);
    }

    #[tokio::test]
    async fn a_target_observer_runs_its_target_and_buffers_nothing() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        let r = s
            .observe(
                &o,
                &sid,
                req("a::b"),
                Delivery::Target("c::run".into()),
                SINK,
            )
            .await
            .unwrap();
        assert_eq!(
            r,
            json!({ "observer_id": "obs-1", "function_id": "a::b", "state": "active",
                    "delivery": "target", "target_function_id": "c::run" })
        );
        assert_eq!(o.asked.lock().unwrap()[0].1, "c::run");
        // The engine never sends its invocations here; a forged one is dropped.
        assert!(!s.deliver(observation("obs-1", json!(1))));
        assert_eq!(s.poll(&sid, None, 50).unwrap()["observations"], json!([]));
        let listed = &s.list(&sid).unwrap()[0];
        assert_eq!(listed["delivery"], "target");
        assert_eq!(listed["target_function_id"], "c::run");
        // Still the session's: unobserve and end dispose it.
        assert!(s.unobserve(&sid, "obs-1").await.is_ok());
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observe_rejects_bad_requests_before_asking_the_engine() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        assert!(s
            .observe(&o, &sid, req(" "), Delivery::Poll, SINK)
            .await
            .unwrap_err()
            .contains("function_id"));
        assert!(s
            .observe(&o, "nope", req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap_err()
            .contains("unknown session"));
        assert!(s
            .observe(&o, &sid, req("a::b"), Delivery::Target(" ".into()), SINK)
            .await
            .unwrap_err()
            .contains("target_function_id"));
        assert!(o.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_session_has_at_most_max_observers() {
        let s = Sessions::new(limits(16, 2, 500));
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        for _ in 0..2 {
            s.observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
                .await
                .unwrap();
        }
        let err = s
            .observe(&o, &sid, req("a::b"), Delivery::Sse, SINK)
            .await
            .unwrap_err();
        assert!(err.contains("2") && err.contains("iii_unobserve"), "{err}");
        assert_eq!(o.asked.lock().unwrap().len(), 2);
        // Another session has its own allowance.
        let other = s.create().unwrap();
        assert!(s
            .observe(&o, &other, req("a::b"), Delivery::Poll, SINK)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_refused_observer_is_disposed_and_reported_with_its_code() {
        let s = store();
        let o = FakeObserve {
            refuse: Some(BusError::new("FORBIDDEN", "not allowed")),
            ..Default::default()
        };
        let sid = s.create().unwrap();
        let err = s
            .observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap_err();
        assert!(
            err.contains("FORBIDDEN") && err.contains("not allowed"),
            "{err}"
        );
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
        assert!(s.list(&sid).unwrap().is_empty());
    }

    #[tokio::test]
    async fn deliveries_are_routed_to_the_owning_session_only() {
        let s = store();
        let o = FakeObserve::default();
        let a = s.create().unwrap();
        let b = s.create().unwrap();
        s.observe(&o, &a, req("x::y"), Delivery::Poll, SINK)
            .await
            .unwrap();
        s.observe(&o, &b, req("x::y"), Delivery::Poll, SINK)
            .await
            .unwrap();
        assert!(s.deliver(observation("obs-1", json!(1))));
        assert!(s.deliver(observation("obs-2", json!(2))));
        assert!(!s.deliver(observation("obs-9", json!(9))));
        assert!(!s.deliver(json!({ "no": "observer id" })));
        assert_eq!(inputs(&s.poll(&a, None, 50).unwrap()), vec![json!(1)]);
        assert_eq!(inputs(&s.poll(&b, None, 50).unwrap()), vec![json!(2)]);
    }

    #[tokio::test]
    async fn the_buffer_is_per_session_oldest_dropped_across_its_observers() {
        let s = Sessions::new(limits(16, 100, 3));
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .unwrap();
        s.deliver(observation("obs-1", json!(1)));
        s.deliver(observation("obs-2", json!("a")));
        s.deliver(observation("obs-1", json!(2)));
        s.deliver(observation("obs-2", json!("b"))); // drops 1
        s.deliver(observation("obs-1", json!(3))); // drops "a"
        let first = s.poll(&sid, None, 2).unwrap();
        assert_eq!(inputs(&first), vec![json!(2), json!("b")]);
        assert_eq!(first["remaining"], 1);
        assert_eq!(first["dropped"], 2);
        let rest = s.poll(&sid, None, 50).unwrap();
        assert_eq!(inputs(&rest), vec![json!(3)]);
        assert_eq!(rest["dropped"], 0);
        // Per observer, for iii_observers.
        let listed = s.list(&sid).unwrap();
        assert_eq!(
            (listed[0]["dropped"].clone(), listed[1]["dropped"].clone()),
            (json!(1), json!(1))
        );
    }

    #[tokio::test]
    async fn poll_reads_one_observer_or_refuses_an_unknown_one() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .unwrap();
        s.deliver(observation("obs-1", json!(1)));
        s.deliver(observation("obs-2", json!(2)));
        let one = s.poll(&sid, Some("obs-2"), 50).unwrap();
        assert_eq!(inputs(&one), vec![json!(2)]);
        assert_eq!(one["remaining"], 0);
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(1)]);
        assert!(s
            .poll(&sid, Some("nope"), 50)
            .unwrap_err()
            .contains("unknown observer"));
        assert!(s
            .poll("nope", None, 50)
            .unwrap_err()
            .contains("unknown session"));
    }

    #[tokio::test]
    async fn list_reports_delivery_counts_and_filters() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        let mut r = req("a::b");
        r.filters = Some(json!({ "input.x": { "$eq": 1 } }));
        s.observe(&o, &sid, r, Delivery::Poll, SINK).await.unwrap();
        s.deliver(observation("obs-1", json!(1)));
        s.deliver(observation("obs-1", json!(2)));
        s.poll(&sid, Some("obs-1"), 1).unwrap();
        assert_eq!(
            s.list(&sid).unwrap(),
            vec![json!({
                "observer_id": "obs-1", "function_id": "a::b", "delivery": "poll",
                "filters": { "input.x": { "$eq": 1 } },
                "state": "active", "buffered": 1, "received": 2, "dropped": 0
            })]
        );
    }

    async fn sse_session(s: &Sessions, o: &FakeObserve) -> String {
        let sid = s.create().unwrap();
        s.observe(o, &sid, req("a::b"), Delivery::Sse, SINK)
            .await
            .unwrap();
        sid
    }

    fn inputs_of(items: &[StreamItem]) -> Vec<Value> {
        items
            .iter()
            .map(|i| match i {
                StreamItem::Observation { observation, .. } => observation["input"].clone(),
                StreamItem::Gap { .. } => json!("gap"),
            })
            .collect()
    }

    fn ids_of(items: &[StreamItem]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|i| match i {
                StreamItem::Observation { id, .. } => Some(*id),
                StreamItem::Gap { .. } => None,
            })
            .collect()
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<StreamItem>) -> Vec<StreamItem> {
        let mut out = vec![];
        while let Ok(item) = rx.try_recv() {
            out.push(item);
        }
        out
    }

    #[tokio::test]
    async fn sse_observations_go_live_to_the_stream_with_ids_and_stay_retained() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .unwrap();
        let (backlog, mut rx) = s.attach_stream(&sid, None).unwrap();
        assert!(backlog.is_empty());
        assert!(s.deliver(observation("obs-1", json!(1))));
        assert!(s.deliver(observation("obs-2", json!(2))));
        let live = drain(&mut rx);
        assert_eq!(inputs_of(&live), vec![json!(1)]);
        // Kept for a reconnect: delivery over the stream is not proof of receipt.
        assert_eq!(s.list(&sid).unwrap()[0]["buffered"], 1);
        // Poll reads only poll observers.
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(2)]);
        assert!(s
            .poll(&sid, Some("obs-1"), 50)
            .unwrap_err()
            .contains("iii_observations_sse"));
    }

    #[tokio::test]
    async fn the_first_stream_starts_with_what_waited_for_it() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        s.deliver(observation("obs-1", json!(1)));
        s.deliver(observation("obs-1", json!(2)));
        let (backlog, mut rx) = s.attach_stream(&sid, None).unwrap();
        assert_eq!(inputs_of(&backlog), vec![json!(1), json!(2)]);
        let ids = ids_of(&backlog);
        assert!(ids[0] < ids[1]);
        assert!(drain(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn a_reconnect_with_last_event_id_replays_everything_after_it() {
        // The client vanished after its socket took 1 and 2: the next write
        // fails, but nothing is lost, because the reconnect replays.
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx) = s.attach_stream(&sid, None).unwrap();
        s.deliver(observation("obs-1", json!(1)));
        s.deliver(observation("obs-1", json!(2)));
        let written = drain(&mut rx);
        for item in &written {
            if let StreamItem::Observation { id, .. } = item {
                s.mark_sent(&sid, *id);
            }
        }
        drop(rx);
        s.deliver(observation("obs-1", json!(3)));
        let first = ids_of(&written)[0];
        let (backlog, _rx) = s.attach_stream(&sid, Some(first)).unwrap();
        assert_eq!(inputs_of(&backlog), vec![json!(2), json!(3)]);
    }

    #[tokio::test]
    async fn a_stream_without_last_event_id_gets_only_what_was_never_written() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx) = s.attach_stream(&sid, None).unwrap();
        s.deliver(observation("obs-1", json!(1)));
        let written = drain(&mut rx);
        s.mark_sent(&sid, ids_of(&written)[0]);
        s.deliver(observation("obs-1", json!(2))); // taken by no stream
        drop(rx);
        let (backlog, _rx) = s.attach_stream(&sid, None).unwrap();
        assert_eq!(inputs_of(&backlog), vec![json!(2)]);
    }

    #[tokio::test]
    async fn a_replay_past_what_is_retained_starts_with_the_gap() {
        let s = Sessions::new(limits(16, 100, 2));
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        for i in 1..=3 {
            s.deliver(observation("obs-1", json!(i)));
        }
        let (backlog, _rx) = s.attach_stream(&sid, Some(0)).unwrap();
        assert_eq!(inputs_of(&backlog), vec![json!("gap"), json!(2), json!(3)]);
        let StreamItem::Gap { after, through } = &backlog[0] else {
            panic!()
        };
        assert_eq!(*after, 0);
        assert!(*through < ids_of(&backlog)[0]);
        // Resuming inside what is retained has no gap.
        let from = ids_of(&backlog)[0];
        let (again, _rx) = s.attach_stream(&sid, Some(from)).unwrap();
        assert_eq!(inputs_of(&again), vec![json!(3)]);
    }

    #[tokio::test]
    async fn a_new_stream_replaces_the_old_one() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut old) = s.attach_stream(&sid, None).unwrap();
        let (_, mut new) = s.attach_stream(&sid, None).unwrap();
        s.deliver(observation("obs-1", json!(1)));
        // The old stream's task sees its channel closed and ends.
        assert!(matches!(
            old.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert_eq!(inputs_of(&drain(&mut new)), vec![json!(1)]);
        assert!(s
            .attach_stream("nope", None)
            .unwrap_err()
            .contains("unknown session"));
    }

    #[tokio::test]
    async fn ending_the_session_ends_its_stream() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx) = s.attach_stream(&sid, None).unwrap();
        assert!(s.end(&sid).await);
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn unobserve_disposes_only_the_sessions_own_observer() {
        let s = store();
        let o = FakeObserve::default();
        let a = s.create().unwrap();
        let b = s.create().unwrap();
        s.observe(&o, &a, req("x::y"), Delivery::Poll, SINK)
            .await
            .unwrap();
        s.deliver(observation("obs-1", json!(1)));
        assert!(s
            .unobserve(&b, "obs-1")
            .await
            .unwrap_err()
            .contains("unknown observer"));
        assert_eq!(o.disposed.load(Ordering::SeqCst), 0);
        let r = s.unobserve(&a, "obs-1").await.unwrap();
        assert_eq!(
            r,
            json!({ "observer_id": "obs-1", "removed": true, "discarded": 1 })
        );
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
        assert!(!s.deliver(observation("obs-1", json!(2))));
        assert_eq!(s.poll(&a, None, 50).unwrap()["observations"], json!([]));
    }

    #[tokio::test]
    async fn ending_a_session_disposes_its_observers() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        for d in [
            Delivery::Poll,
            Delivery::Sse,
            Delivery::Target("c::run".into()),
        ] {
            s.observe(&o, &sid, req("a::b"), d, SINK).await.unwrap();
        }
        assert!(s.end(&sid).await);
        assert_eq!(o.disposed.load(Ordering::SeqCst), 3);
        assert!(!s.end(&sid).await);
        assert!(!s.deliver(observation("obs-1", json!(1))));
    }

    #[tokio::test]
    async fn idle_sessions_are_ended_and_active_ones_kept() {
        let s = Sessions::new(SessionLimits {
            idle_ttl: Duration::from_millis(20),
            ..SessionLimits::default()
        });
        let o = FakeObserve::default();
        let idle = s.create().unwrap();
        let busy = s.create().unwrap();
        s.observe(&o, &idle, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(s.touch(&busy));
        assert_eq!(s.end_idle().await, 1);
        assert!(!s.touch(&idle));
        assert!(s.touch(&busy));
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn close_all_disposes_everything() {
        let s = store();
        let o = FakeObserve::default();
        for _ in 0..2 {
            let sid = s.create().unwrap();
            s.observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
                .await
                .unwrap();
        }
        s.close_all().await;
        assert_eq!(o.disposed.load(Ordering::SeqCst), 2);
    }
}
