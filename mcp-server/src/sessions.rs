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
use serde_json::{Value, json};
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
    /// `max_observations` (0 is no limit, as for the engine): once
    /// `received` reaches it, every delivery came.
    limit: Option<u64>,
}

impl Entry {
    /// Ended, with every delivery accounted for: a target observer (its
    /// deliveries bypass the session), or all of its `max_observations`
    /// arrived. Anything else (an expiry, a limit not
    /// reached) may still have an observation in flight, with no bound on
    /// when it lands, so only `iii_unobserve` frees it.
    fn complete(&self) -> bool {
        if self.handle.state() != "disposed" {
            return false;
        }
        match self.delivery {
            // Its observations go to the target, never through the
            // session: ending it here drops nothing in flight.
            Delivery::Target(_) => true,
            _ => self.limit.is_some_and(|m| self.received >= m),
        }
    }
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
    /// Bounded: a stream that falls behind is let go (its client resumes
    /// with `Last-Event-ID`), so a slow reader cannot grow memory.
    stream: Option<mpsc::Sender<StreamItem>>,
    /// The profile this session follows: the last one loaded with
    /// `iii_get_agent { activate: true }`.
    active_profile: Option<String>,
    /// Which stream owns the session's SSE: a new `GET` takes it over, and
    /// the writer of the one it replaced stops before its next frame.
    stream_gen: u64,
    /// The highest SSE event id a stream wrote.
    sent_through: u64,
    /// The highest SSE event id the cap dropped.
    sse_dropped_through: u64,
    /// The observations streams claimed and are writing now: at most one
    /// per stream generation (a writer sends one frame at a time), so a
    /// stream that took over cannot erase the replaced one's record.
    in_flight: Vec<InFlight>,
    /// SSE observations whose write failed after a later one was sent, so
    /// `sent_through` cannot be rolled back over them. Every backlog carries
    /// them, whatever its `Last-Event-ID`, until the client shows it has
    /// one (see `written`) or the cap drops it. A write does not settle
    /// it: the client can drop before reading. Value: each write of it
    /// since it became owed, as (stream generation, write order).
    unsent: std::collections::BTreeMap<u64, Vec<(u64, u64)>>,
    /// Every write of each buffered SSE observation: (stream generation,
    /// write order). A client resuming from `Last-Event-ID` L got L from one
    /// of the streams that wrote it, so an owed observation is settled only
    /// if every one of them wrote it before L.
    written: HashMap<u64, Vec<(u64, u64)>>,
    /// Counts successful writes, for `written`.
    write_order: u64,
}

/// A claimed observation whose write has not reported yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InFlight {
    generation: u64,
    id: u64,
    /// `sent_through` before the claim, restored when the write fails.
    prev: u64,
    /// A stream that took over sent it in its backlog (a replay from an
    /// earlier `Last-Event-ID`), so a failed write needs no resend.
    covered: bool,
}

/// The observation event ids in a stream backlog.
fn backlog_ids_of(backlog: &[StreamItem]) -> Vec<u64> {
    backlog
        .iter()
        .filter_map(|i| match i {
            StreamItem::Observation { id, .. } => Some(*id),
            StreamItem::Gap { .. } => None,
        })
        .collect()
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
        if let Some(stream) = &self.stream
            && stream.try_send(item).is_err()
        {
            self.stream = None;
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
                active_profile: None,
                stream_gen: 0,
                sent_through: 0,
                sse_dropped_through: 0,
                in_flight: Vec::new(),
                unsent: std::collections::BTreeMap::new(),
                written: HashMap::new(),
                write_order: 0,
            },
        );
        Ok(id)
    }

    /// Open a session for `initialize`. At `max_sessions`, the session seen
    /// least recently is ended to make room (a client that cannot end its
    /// own, such as a headless Claude Code turn that exits, would otherwise
    /// lock every client out until the idle TTL); its client re-initializes
    /// on the 404 its next request gets. A session with an open stream is
    /// never ended this way. Returns the new id and the ended one.
    pub async fn open(&self) -> Result<(String, Option<String>), String> {
        let evicted = {
            let inner = self.inner.lock().unwrap();
            if inner.sessions.len() < self.limits.max_sessions {
                None
            } else {
                let victim = inner
                    .sessions
                    .iter()
                    .filter(|(_, s)| !s.stream.as_ref().is_some_and(|tx| !tx.is_closed()))
                    .min_by_key(|(_, s)| s.last_seen)
                    .map(|(id, _)| id.clone());
                match victim {
                    Some(id) => Some(id),
                    None => {
                        return Err(format!(
                            "this MCP server already has {} sessions (max_sessions), each with an open stream; end one with DELETE",
                            self.limits.max_sessions
                        ));
                    }
                }
            }
        };
        if let Some(id) = &evicted {
            self.end(id).await;
        }
        Ok((self.create()?, evicted))
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

    /// Record the profile the session follows; `false` when it does not exist.
    pub fn set_active_profile(&self, session_id: &str, profile_id: &str) -> bool {
        match self.inner.lock().unwrap().sessions.get_mut(session_id) {
            Some(s) => {
                s.active_profile = Some(profile_id.to_string());
                true
            }
            None => false,
        }
    }

    /// The profile the session activated, if any (`iii_status` reports the
    /// default profile otherwise).
    pub fn active_profile(&self, session_id: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .get(session_id)
            .and_then(|s| s.active_profile.clone())
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
                return Err("target_function_id must name a function".into());
            }
            Delivery::Target(t) => Delivery::Target(t.trim().to_string()),
            other => other,
        };
        let target = match &delivery {
            Delivery::Target(t) => t.as_str(),
            _ => sink,
        };
        // The SDK sends the observe request before it returns the handle, so
        // the engine can deliver before the observer is recorded. The routing
        // lock is held from before the handle exists until it is recorded: a
        // delivery that races it waits, then finds its session. Creating the
        // handle does not wait on anything, so this holds no lock across an
        // await.
        let (observer_id, handle) = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(session_id) else {
                return Err(format!("unknown session: {session_id}"));
            };
            session.last_seen = Instant::now();
            if session.observers.len() >= self.limits.max_observers {
                // A finished observer holds its place until every delivery
                // arrived (`Entry::complete`) and none is still buffered:
                // unpolled, or kept for SSE replay.
                let finished: Vec<String> = session
                    .observers
                    .iter()
                    .filter(|(id, e)| e.complete() && session.buffered_for(id) == 0)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in &finished {
                    session.observers.remove(id);
                    session.order.retain(|o| o != id);
                }
                for id in &finished {
                    inner.owner.remove(id);
                }
            }
            let session = inner
                .sessions
                .get_mut(session_id)
                .expect("the session cannot end while the lock is held");
            if session.observers.len() >= self.limits.max_observers {
                return Err(format!(
                    "this session already has {} observers (max_observers_per_session); remove one with iii_unobserve",
                    self.limits.max_observers
                ));
            }
            let handle: Arc<dyn ObserverLike> = Arc::from(observe.observe(
                &ObserveRequest {
                    function_id: function_id.clone(),
                    ..req.clone()
                },
                target,
            ));
            let observer_id = handle.id();
            let session = inner
                .sessions
                .get_mut(session_id)
                .expect("the session cannot end while the lock is held");
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
                    limit: req.max_observations.filter(|m| *m > 0),
                },
            );
            inner
                .owner
                .insert(observer_id.clone(), session_id.to_string());
            (observer_id, handle)
        };
        if let Ok(Err(e)) = tokio::time::timeout(READY_WAIT, handle.ready()).await {
            // `disposed` is an observer that already ended: it fired its
            // max_observations (or expired) before activation was confirmed.
            // Its observations stay. Only `error` is a refusal.
            if handle.state() != "disposed" {
                self.forget(session_id, &observer_id);
                handle.dispose().await;
                return Err(format!("observe refused: {}: {}", e.code, e.message));
            }
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
        // An observation of another function under this observer's id is
        // forged: the engine names the function it observed.
        if let Some(function_id) = observation.get("function_id").and_then(Value::as_str)
            && function_id != entry.function_id
        {
            return false;
        }
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
                let lost = if session.is_sse(&id) {
                    // Out of the replay window: a gap for a client resuming
                    // from before it, a loss only if no stream wrote it.
                    session.sse_dropped_through = session.sse_dropped_through.max(dropped_seq);
                    // An owed one was never written either.
                    let owed = session.unsent.remove(&dropped_seq).is_some();
                    session.written.remove(&dropped_seq);
                    owed || dropped_seq > session.sent_through
                } else {
                    session.dropped += 1;
                    true
                };
                if lost && let Some(e) = session.observers.get_mut(&id) {
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
                    ));
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
    ) -> Result<(Vec<StreamItem>, mpsc::Receiver<StreamItem>, u64), String> {
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        session.last_seen = Instant::now();
        // The client has `last_event_id` and, the stream being ordered,
        // everything its stream wrote before it: those owed are settled.
        if let Some(l) = last_event_id {
            // Every place the client may have got L from: each completed
            // write (stream, order), and each write still in flight (stream,
            // no order yet: it comes after everything that stream finished).
            let mut sources: Vec<(u64, Option<u64>)> = session
                .written
                .get(&l)
                .map(|w| w.iter().map(|(g, o)| (*g, Some(*o))).collect())
                .unwrap_or_default();
            sources.extend(
                session
                    .in_flight
                    .iter()
                    .filter(|f| f.id == l)
                    .map(|f| (f.generation, None)),
            );
            if !sources.is_empty() {
                // Settled only if every possible source wrote it first.
                session.unsent.retain(|_, writes| {
                    !sources.iter().all(|(g, o)| {
                        writes
                            .iter()
                            .any(|(gx, ox)| gx == g && o.is_none_or(|o| *ox <= o))
                    })
                });
            }
        }
        let after = last_event_id.unwrap_or(session.sent_through);
        let mut backlog = Vec::new();
        if session.sse_dropped_through > after {
            backlog.push(StreamItem::Gap {
                after,
                through: session.sse_dropped_through,
            });
        }
        for (seq, id, observation) in &session.buffer {
            if (*seq > after || session.unsent.contains_key(seq)) && session.is_sse(id) {
                backlog.push(StreamItem::Observation {
                    id: *seq,
                    observation: observation.clone(),
                });
            }
        }
        // The replaced stream's write in flight: if this backlog carries it,
        // a failure there needs no resend here.
        let ids = backlog_ids_of(&backlog);
        for f in session.in_flight.iter_mut() {
            f.covered = f.covered || ids.contains(&f.id);
        }
        let (tx, rx) = mpsc::channel(self.limits.max_buffered.max(1));
        session.stream = Some(tx);
        session.stream_gen += 1;
        Ok((backlog, rx, session.stream_gen))
    }

    /// Whether the stream `generation` still owns the session's SSE; a
    /// writer stops once it does not, so two streams never send one message.
    pub fn is_current_stream(&self, session_id: &str, generation: u64) -> bool {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .get(session_id)
            .is_some_and(|s| s.stream_gen == generation)
    }

    /// Claim the next item for the stream `generation`: in one step under
    /// the session lock, check that the stream still owns the session's SSE
    /// and record observation `id` as sent. A stream that attaches afterwards
    /// leaves it out of its backlog; one that attached before makes the claim
    /// fail, and sends it itself. `false`: stop writing.
    pub fn claim_stream_item(&self, session_id: &str, generation: u64, id: Option<u64>) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(session) = inner.sessions.get_mut(session_id) else {
            return false;
        };
        if session.stream_gen != generation {
            return false;
        }
        if let Some(id) = id {
            session.in_flight.retain(|f| f.generation != generation);
            session.in_flight.push(InFlight {
                generation,
                id,
                prev: session.sent_through,
                covered: false,
            });
            session.sent_through = session.sent_through.max(id);
        }
        true
    }

    /// The stream `generation` finished writing claimed observation `id`.
    /// A failed write is unsent again: the next `GET` without
    /// `Last-Event-ID` sends it, and a stream that took over meanwhile (and
    /// left it out of its backlog as claimed) gets it now.
    pub fn finish_stream_item(&self, session_id: &str, generation: u64, id: u64, written: bool) {
        let mut inner = self.inner.lock().unwrap();
        let Some(session) = inner.sessions.get_mut(session_id) else {
            return;
        };
        let Some(at) = session
            .in_flight
            .iter()
            .position(|f| f.generation == generation && f.id == id)
        else {
            return;
        };
        let f = session.in_flight.remove(at);
        if written {
            session.write_order += 1;
            let at = (generation, session.write_order);
            session.written.entry(id).or_default().push(at);
            if let Some(w) = session.unsent.get_mut(&id) {
                w.push(at);
            }
            return;
        }
        if session.sent_through == id {
            session.sent_through = f.prev;
        } else {
            // A later event already counts as sent: keep this one owed.
            session.unsent.entry(id).or_default();
        }
        if session.stream_gen != generation && !f.covered {
            let observation = session
                .buffer
                .iter()
                .find(|(seq, _, _)| *seq == id)
                .map(|(_, _, o)| o.clone());
            if let Some(observation) = observation {
                session.push(StreamItem::Observation { id, observation });
            }
        }
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
        let candidates: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner
                .sessions
                .iter()
                .filter(|(_, s)| s.last_seen.elapsed() > self.limits.idle_ttl)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut n = 0;
        for id in candidates {
            if self.end_if_idle(&id).await {
                n += 1;
            }
        }
        n
    }

    /// End `session_id` only if it is still idle: the check and the removal
    /// happen under one lock, so a request that refreshed it in between wins.
    pub async fn end_if_idle(&self, session_id: &str) -> bool {
        let handles: Vec<Arc<dyn ObserverLike>> = {
            let mut inner = self.inner.lock().unwrap();
            let idle = inner
                .sessions
                .get(session_id)
                .is_some_and(|s| s.last_seen.elapsed() > self.limits.idle_ttl);
            if !idle {
                return false;
            }
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// An observer the engine accepted (or refused) at once.
    struct FakeObserver {
        id: String,
        refused: Option<BusError>,
        /// Reached its fire limit before activation was confirmed.
        fired: bool,
        disposed: Arc<AtomicUsize>,
    }

    impl ObserverLike for FakeObserver {
        fn id(&self) -> String {
            self.id.clone()
        }
        fn state(&self) -> String {
            if self.fired {
                "disposed".into()
            } else if self.refused.is_some() {
                "error".into()
            } else {
                "active".into()
            }
        }
        fn ready(&self) -> BoxFuture<'_, Result<(), BusError>> {
            let r = if self.fired {
                Some(BusError::new("UNOBSERVED", "observer is disposed"))
            } else {
                self.refused.clone()
            };
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
        fire_early: bool,
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
                fired: self.fire_early,
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
        json!({ "observer_id": observer_id, "status": "success", "input": input })
    }

    fn req(function_id: &str) -> ObserveRequest {
        serde_json::from_value(json!({ "function_id": function_id })).unwrap()
    }

    fn req_limited(function_id: &str, max_observations: u64) -> ObserveRequest {
        serde_json::from_value(
            json!({ "function_id": function_id, "max_observations": max_observations }),
        )
        .unwrap()
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
        assert!(
            s.observe(&o, &sid, req(" "), Delivery::Poll, SINK)
                .await
                .unwrap_err()
                .contains("function_id")
        );
        assert!(
            s.observe(&o, "nope", req("a::b"), Delivery::Poll, SINK)
                .await
                .unwrap_err()
                .contains("unknown session")
        );
        assert!(
            s.observe(&o, &sid, req("a::b"), Delivery::Target(" ".into()), SINK)
                .await
                .unwrap_err()
                .contains("target_function_id")
        );
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
        assert!(
            s.observe(&o, &other, req("a::b"), Delivery::Poll, SINK)
                .await
                .is_ok()
        );
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
        assert!(
            s.poll(&sid, Some("nope"), 50)
                .unwrap_err()
                .contains("unknown observer")
        );
        assert!(
            s.poll("nope", None, 50)
                .unwrap_err()
                .contains("unknown session")
        );
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

    fn drain(rx: &mut mpsc::Receiver<StreamItem>) -> Vec<StreamItem> {
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
        let (backlog, mut rx, _) = s.attach_stream(&sid, None).unwrap();
        assert!(backlog.is_empty());
        assert!(s.deliver(observation("obs-1", json!(1))));
        assert!(s.deliver(observation("obs-2", json!(2))));
        let live = drain(&mut rx);
        assert_eq!(inputs_of(&live), vec![json!(1)]);
        // Kept for a reconnect: delivery over the stream is not proof of receipt.
        assert_eq!(s.list(&sid).unwrap()[0]["buffered"], 1);
        // Poll reads only poll observers.
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(2)]);
        assert!(
            s.poll(&sid, Some("obs-1"), 50)
                .unwrap_err()
                .contains("iii_observations_sse")
        );
    }

    #[tokio::test]
    async fn the_first_stream_starts_with_what_waited_for_it() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        s.deliver(observation("obs-1", json!(1)));
        s.deliver(observation("obs-1", json!(2)));
        let (backlog, mut rx, _) = s.attach_stream(&sid, None).unwrap();
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
        let (_, mut rx, _) = s.attach_stream(&sid, None).unwrap();
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
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(first)).unwrap();
        assert_eq!(inputs_of(&backlog), vec![json!(2), json!(3)]);
    }

    #[tokio::test]
    async fn a_stream_without_last_event_id_gets_only_what_was_never_written() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx, _) = s.attach_stream(&sid, None).unwrap();
        s.deliver(observation("obs-1", json!(1)));
        let written = drain(&mut rx);
        s.mark_sent(&sid, ids_of(&written)[0]);
        s.deliver(observation("obs-1", json!(2))); // taken by no stream
        drop(rx);
        let (backlog, _rx, _) = s.attach_stream(&sid, None).unwrap();
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
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(0)).unwrap();
        assert_eq!(inputs_of(&backlog), vec![json!("gap"), json!(2), json!(3)]);
        let StreamItem::Gap { after, through } = &backlog[0] else {
            panic!()
        };
        assert_eq!(*after, 0);
        assert!(*through < ids_of(&backlog)[0]);
        // Resuming inside what is retained has no gap.
        let from = ids_of(&backlog)[0];
        let (again, _rx, _) = s.attach_stream(&sid, Some(from)).unwrap();
        assert_eq!(inputs_of(&again), vec![json!(3)]);
    }

    #[tokio::test]
    async fn a_new_stream_replaces_the_old_one() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut old, _) = s.attach_stream(&sid, None).unwrap();
        let (_, mut new, _) = s.attach_stream(&sid, None).unwrap();
        s.deliver(observation("obs-1", json!(1)));
        // The old stream's task sees its channel closed and ends.
        assert!(matches!(
            old.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        assert_eq!(inputs_of(&drain(&mut new)), vec![json!(1)]);
        assert!(
            s.attach_stream("nope", None)
                .unwrap_err()
                .contains("unknown session")
        );
    }

    #[tokio::test]
    async fn ending_the_session_ends_its_stream() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx, _) = s.attach_stream(&sid, None).unwrap();
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
        assert!(
            s.unobserve(&b, "obs-1")
                .await
                .unwrap_err()
                .contains("unknown observer")
        );
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
    async fn an_observer_that_fired_before_activation_is_kept_not_refused() {
        // max_observations: 1 can be spent before ready() returns: the
        // observer is disposed, not refused, and its observation stays.
        let s = store();
        let o = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        let r = s
            .observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        assert_eq!(r["state"], "disposed");
        assert!(s.deliver(observation("obs-1", json!(1))));
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(1)]);
        assert_eq!(s.list(&sid).unwrap()[0]["state"], "disposed");
    }

    #[tokio::test]
    async fn a_delivery_must_name_the_observed_function() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(&o, &sid, req("state::set"), Delivery::Poll, SINK)
            .await
            .unwrap();
        let forged = json!({ "observer_id": "obs-1", "function_id": "evil::own", "input": 1 });
        assert!(!s.deliver(forged));
        assert!(s.deliver(observation("obs-1", json!(2))));
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(2)]);
    }

    #[tokio::test]
    async fn a_stream_that_falls_behind_is_closed_and_nothing_is_lost() {
        let s = Sessions::new(limits(16, 100, 3));
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(&o, &sid, req("state::set"), Delivery::Sse, SINK)
            .await
            .unwrap();
        let (_, mut rx, _) = s.attach_stream(&sid, None).unwrap();
        // Nobody reads: the queue is bounded, so the stream is let go.
        for i in 1..=5 {
            assert!(s.deliver(observation("obs-1", json!(i))));
        }
        let got = drain(&mut rx);
        assert!(got.len() <= 3, "bounded: {}", got.len());
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        // The reconnect replays what the buffer still holds.
        let last = ids_of(&got).first().copied().unwrap_or(0);
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(last)).unwrap();
        let replayed = inputs_of(&backlog);
        assert!(
            replayed.ends_with(&[json!(3), json!(4), json!(5)]),
            "{replayed:?}"
        );
    }

    #[tokio::test]
    async fn the_idle_sweep_spares_a_session_refreshed_in_the_meantime() {
        let s = Sessions::new(SessionLimits {
            idle_ttl: Duration::from_millis(20),
            ..SessionLimits::default()
        });
        let sid = s.create().unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        // The sweep saw it idle, then a request refreshed it before removal.
        assert!(s.touch(&sid));
        assert!(!s.end_if_idle(&sid).await);
        assert!(s.touch(&sid));
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(s.end_if_idle(&sid).await);
    }

    #[tokio::test]
    async fn sse_replay_eviction_is_not_a_poll_drop() {
        let s = Sessions::new(limits(16, 100, 2));
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(&o, &sid, req("state::set"), Delivery::Sse, SINK)
            .await
            .unwrap();
        let (_, mut rx, _) = s.attach_stream(&sid, None).unwrap();
        for i in 1..=3 {
            s.deliver(observation("obs-1", json!(i)));
            for id in ids_of(&drain(&mut rx)) {
                s.mark_sent(&sid, id);
            }
        }
        // Evicting an observation a stream already wrote loses nothing.
        assert_eq!(s.poll(&sid, None, 50).unwrap()["dropped"], 0);
        assert_eq!(s.list(&sid).unwrap()[0]["dropped"], 0);
        // One no stream wrote is a loss for that observer, still not a poll drop.
        drop(rx);
        for i in 4..=6 {
            s.deliver(observation("obs-1", json!(i)));
        }
        assert_eq!(s.poll(&sid, None, 50).unwrap()["dropped"], 0);
        assert_eq!(s.list(&sid).unwrap()[0]["dropped"], 1);
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

    #[tokio::test]
    async fn a_full_server_ends_the_least_recently_used_session_for_a_new_one() {
        let s = Sessions::new(limits(2, 100, 500));
        let o = FakeObserve::default();
        let old = s.create().unwrap();
        s.observe(&o, &old, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let recent = s.create().unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(s.touch(&recent));
        let (fresh, evicted) = s.open().await.unwrap();
        assert_eq!(evicted.as_deref(), Some(old.as_str()));
        assert!(!s.touch(&old), "the evicted session is gone");
        assert!(s.touch(&recent));
        assert!(s.touch(&fresh));
        // Its observers are disposed like on DELETE.
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn opening_below_the_limit_ends_nothing() {
        let s = Sessions::new(limits(2, 100, 500));
        let a = s.create().unwrap();
        let (b, evicted) = s.open().await.unwrap();
        assert_eq!(evicted, None);
        assert!(s.touch(&a) && s.touch(&b));
    }

    #[tokio::test]
    async fn a_session_with_an_open_stream_is_never_evicted() {
        let s = Sessions::new(limits(2, 100, 500));
        let streaming = s.create().unwrap();
        let _rx = s.attach_stream(&streaming, None).unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let idle = s.create().unwrap();
        let (_, evicted) = s.open().await.unwrap();
        assert_eq!(
            evicted.as_deref(),
            Some(idle.as_str()),
            "the older one streams"
        );
        assert!(s.touch(&streaming));
    }

    #[tokio::test]
    async fn when_every_session_streams_a_new_one_is_refused() {
        let s = Sessions::new(limits(1, 100, 500));
        let only = s.create().unwrap();
        let _rx = s.attach_stream(&only, None).unwrap();
        let err = s.open().await.unwrap_err();
        assert!(err.contains("max_sessions"), "{err}");
        assert!(err.contains("stream"), "{err}");
        assert!(s.touch(&only));
    }

    /// Delivers one observation from another thread while `observe` is still
    /// creating the handle, as the engine can once the SDK sent the request.
    struct EarlyDelivery {
        sessions: Arc<Sessions>,
    }

    impl Observe for EarlyDelivery {
        fn observe(&self, _req: &ObserveRequest, _target: &str) -> Box<dyn ObserverLike> {
            let sessions = self.sessions.clone();
            let early = std::thread::spawn(move || {
                sessions.deliver(observation("obs-early", json!("first")))
            });
            // The handle is not back yet when the delivery arrives.
            std::thread::sleep(Duration::from_millis(50));
            Box::leak(Box::new(early));
            Box::new(FakeObserver {
                id: "obs-early".into(),
                refused: None,
                fired: false,
                disposed: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    #[tokio::test]
    async fn a_delivery_that_races_the_observe_call_is_kept() {
        let s = Arc::new(store());
        let sid = s.create().unwrap();
        let o = EarlyDelivery {
            sessions: s.clone(),
        };
        s.observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        // The racing delivery waited for the observer to be routed.
        let mut polled = json!([]);
        for _ in 0..50 {
            polled = s.poll(&sid, None, 50).unwrap()["observations"].clone();
            if polled.as_array().is_some_and(|a| !a.is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            inputs(&json!({ "observations": polled })),
            vec![json!("first")]
        );
    }

    #[test]
    fn a_new_stream_replaces_the_old_ones_writer() {
        let s = store();
        let sid = s.create().unwrap();
        let (_, _rx_a, a) = s.attach_stream(&sid, None).unwrap();
        assert!(s.is_current_stream(&sid, a));
        let (_, _rx_b, b) = s.attach_stream(&sid, None).unwrap();
        assert!(!s.is_current_stream(&sid, a), "the old writer must stop");
        assert!(s.is_current_stream(&sid, b));
    }

    /// The observation ids in a stream backlog.
    fn backlog_ids(items: &[StreamItem]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|i| match i {
                StreamItem::Observation { id, .. } => Some(*id),
                StreamItem::Gap { .. } => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_writer_that_claimed_an_observation_keeps_it_from_the_next_stream() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx_a, a) = s.attach_stream(&sid, None).unwrap();
        assert!(s.deliver(observation("obs-1", json!(1))));
        let StreamItem::Observation { id, .. } = rx_a.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        // A claims it (ownership check and "sent" in one step), then B
        // attaches before A has written it: B must not send it too.
        assert!(s.claim_stream_item(&sid, a, Some(id)));
        let (backlog_b, _rx_b, _) = s.attach_stream(&sid, None).unwrap();
        assert!(
            !backlog_ids(&backlog_b).contains(&id),
            "both streams would write {id}"
        );
    }

    #[tokio::test]
    async fn a_writer_replaced_before_its_claim_leaves_the_observation_to_the_new_stream() {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx_a, a) = s.attach_stream(&sid, None).unwrap();
        assert!(s.deliver(observation("obs-1", json!(1))));
        let StreamItem::Observation { id, .. } = rx_a.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        // B attaches between A receiving it and A claiming it.
        let (backlog_b, _rx_b, b) = s.attach_stream(&sid, None).unwrap();
        assert!(
            !s.claim_stream_item(&sid, a, Some(id)),
            "A no longer owns the stream"
        );
        assert!(backlog_ids(&backlog_b).contains(&id), "B sends it instead");
        assert!(s.claim_stream_item(&sid, b, Some(id)));
    }

    /// A's first observation, claimed by A: (store, session, A's generation, id, A's receiver).
    async fn claimed_by_a() -> (Sessions, String, u64, u64, mpsc::Receiver<StreamItem>) {
        let s = store();
        let o = FakeObserve::default();
        let sid = sse_session(&s, &o).await;
        let (_, mut rx_a, a) = s.attach_stream(&sid, None).unwrap();
        assert!(s.deliver(observation("obs-1", json!(1))));
        let StreamItem::Observation { id, .. } = rx_a.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        assert!(s.claim_stream_item(&sid, a, Some(id)));
        (s, sid, a, id, rx_a)
    }

    #[tokio::test]
    async fn a_failed_write_leaves_the_observation_for_the_next_stream() {
        let (s, sid, a, id, _rx_a) = claimed_by_a().await;
        s.finish_stream_item(&sid, a, id, false);
        let (backlog, _rx, _) = s.attach_stream(&sid, None).unwrap();
        assert!(backlog_ids(&backlog).contains(&id), "no stream wrote {id}");
    }

    #[tokio::test]
    async fn a_written_observation_is_not_sent_again() {
        let (s, sid, a, id, _rx_a) = claimed_by_a().await;
        s.finish_stream_item(&sid, a, id, true);
        let (backlog, _rx, _) = s.attach_stream(&sid, None).unwrap();
        assert!(!backlog_ids(&backlog).contains(&id));
    }

    #[tokio::test]
    async fn a_write_that_fails_after_a_takeover_reaches_the_new_stream() {
        let (s, sid, a, id, _rx_a) = claimed_by_a().await;
        // B attaches while A's write is in flight: claimed, so not in its backlog.
        let (backlog_b, mut rx_b, _) = s.attach_stream(&sid, None).unwrap();
        assert!(!backlog_ids(&backlog_b).contains(&id));
        s.finish_stream_item(&sid, a, id, false);
        match rx_b.try_recv() {
            Ok(StreamItem::Observation { id: got, .. }) => assert_eq!(got, id),
            other => panic!("B must send {id}, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_write_that_succeeds_after_a_takeover_is_not_resent() {
        let (s, sid, a, id, _rx_a) = claimed_by_a().await;
        let (_, mut rx_b, _) = s.attach_stream(&sid, None).unwrap();
        s.finish_stream_item(&sid, a, id, true);
        assert!(rx_b.try_recv().is_err(), "A wrote it");
    }

    #[tokio::test]
    async fn a_replay_from_an_earlier_event_id_is_not_sent_twice_after_a_failed_write() {
        let (s, sid, a, id, _rx_a) = claimed_by_a().await;
        // The client resumes from before `id`: B replays it, as asked.
        let (backlog_b, mut rx_b, _) = s.attach_stream(&sid, Some(0)).unwrap();
        assert!(backlog_ids(&backlog_b).contains(&id));
        s.finish_stream_item(&sid, a, id, false);
        assert!(rx_b.try_recv().is_err(), "already in B's backlog");
    }

    #[tokio::test]
    async fn a_finished_observer_frees_its_place_once_its_observations_are_read() {
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        s.observe(&fired, &sid, req_limited("a::b", 1), Delivery::Poll, SINK)
            .await
            .unwrap();
        assert!(s.deliver(observation("obs-1", json!(1))));
        // Unread: it still holds its place, so its observation is kept.
        let o = FakeObserve::default();
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err()
        );
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(1)]);
        // Read and finished: its place is free.
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .expect("the finished observer no longer counts");
        let listed = s.list(&sid).unwrap();
        assert_eq!(listed.len(), 1, "only the new observer: {listed:?}");
        assert_eq!(listed[0]["function_id"], "c::d");
    }

    #[tokio::test]
    async fn a_finished_sse_observer_keeps_its_place_while_its_replay_is_buffered() {
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        s.observe(&fired, &sid, req("a::b"), Delivery::Sse, SINK)
            .await
            .unwrap();
        assert!(s.deliver(observation("obs-1", json!(1))));
        let o = FakeObserve::default();
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err()
        );
        // A resuming client still gets it.
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(0)).unwrap();
        assert_eq!(backlog_ids(&backlog).len(), 1);
    }

    #[tokio::test]
    async fn an_active_observer_keeps_its_place() {
        let s = Sessions::new(limits(16, 1, 500));
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(&o, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_finished_observer_keeps_its_place_until_its_last_delivery_arrives() {
        // The engine can report the observer disposed while its last
        // observation is still on its way to the sink.
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        s.observe(&fired, &sid, req_limited("a::b", 1), Delivery::Poll, SINK)
            .await
            .unwrap();
        let o = FakeObserve {
            next: AtomicUsize::new(100),
            ..Default::default()
        };
        // Nothing received yet: its place (and its route) stay.
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err()
        );
        assert!(
            s.deliver(observation("obs-1", json!(1))),
            "the late delivery still lands"
        );
        assert_eq!(inputs(&s.poll(&sid, None, 50).unwrap()), vec![json!(1)]);
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .expect("all of it arrived and was read");
    }

    #[tokio::test]
    async fn a_finished_observer_with_no_known_end_keeps_its_place() {
        // Neither max_observations nor an expiry long past says every
        // delivery arrived, so it is not freed behind the client's back.
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        s.observe(&fired, &sid, req("a::b"), Delivery::Poll, SINK)
            .await
            .unwrap();
        let o = FakeObserve {
            next: AtomicUsize::new(100),
            ..Default::default()
        };
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_takeover_writing_its_own_item_keeps_the_old_write_recoverable() {
        // A claims 1; B attaches (1 is claimed, so not in its backlog) and
        // claims 2; then A's write of 1 fails. B must still get 1.
        let (s, sid, a, one, _rx_a) = claimed_by_a().await;
        let (backlog_b, mut rx_b, b) = s.attach_stream(&sid, None).unwrap();
        assert!(!backlog_ids(&backlog_b).contains(&one));
        assert!(s.deliver(observation("obs-1", json!(2))));
        let StreamItem::Observation { id: two, .. } = rx_b.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        assert!(s.claim_stream_item(&sid, b, Some(two)));
        s.finish_stream_item(&sid, a, one, false);
        s.finish_stream_item(&sid, b, two, true);
        match rx_b.try_recv() {
            Ok(StreamItem::Observation { id, .. }) => assert_eq!(id, one),
            other => panic!("B must send {one}, got {other:?}"),
        }
    }

    /// A finished observer at a limit of one, and the second observe it blocks.
    async fn blocked_by(r: ObserveRequest) -> bool {
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        s.observe(&fired, &sid, r, Delivery::Poll, SINK)
            .await
            .unwrap();
        let o = FakeObserve {
            next: AtomicUsize::new(100),
            ..Default::default()
        };
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .is_err()
    }

    #[tokio::test]
    async fn a_zero_max_observations_is_no_limit() {
        // The engine ignores 0, so nothing says every delivery arrived.
        let r = serde_json::from_value(json!({ "function_id": "a::b", "max_observations": 0 }))
            .unwrap();
        assert!(blocked_by(r).await, "its place is kept");
    }

    #[tokio::test]
    async fn an_expired_observer_keeps_its_place_until_unobserved() {
        // No bound on delivery time says the last observation arrived;
        // iii_unobserve frees it.
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        let r =
            serde_json::from_value(json!({ "function_id": "a::b", "expires_in_ms": 1 })).unwrap();
        s.observe(&fired, &sid, r, Delivery::Poll, SINK)
            .await
            .unwrap();
        let o = FakeObserve {
            next: AtomicUsize::new(100),
            ..Default::default()
        };
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err(),
            "expired, but its place is kept"
        );
        s.unobserve(&sid, "obs-1").await.unwrap();
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .expect("iii_unobserve freed it");
    }

    #[tokio::test]
    async fn a_finished_target_observer_frees_its_place() {
        // Its observations go straight to the target, never through the
        // session, so ending it here drops nothing in flight.
        let s = Sessions::new(limits(16, 1, 500));
        let fired = FakeObserve {
            fire_early: true,
            ..Default::default()
        };
        let sid = s.create().unwrap();
        s.observe(
            &fired,
            &sid,
            req_limited("a::b", 1),
            Delivery::Target("my::target".into()),
            SINK,
        )
        .await
        .unwrap();
        let o = FakeObserve {
            next: AtomicUsize::new(100),
            ..Default::default()
        };
        s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
            .await
            .expect("the finished target observer no longer counts");
    }

    #[tokio::test]
    async fn an_active_target_observer_keeps_its_place() {
        let s = Sessions::new(limits(16, 1, 500));
        let o = FakeObserve::default();
        let sid = s.create().unwrap();
        s.observe(
            &o,
            &sid,
            req("a::b"),
            Delivery::Target("my::target".into()),
            SINK,
        )
        .await
        .unwrap();
        assert!(
            s.observe(&o, &sid, req("c::d"), Delivery::Poll, SINK)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_requeued_observation_survives_a_disconnect_and_a_later_event_id() {
        // A claims 1; B takes over and writes 2; A's write of 1 fails and 1
        // is queued for B; B drops before writing it. The client resumes
        // from 2: 1 must still come, until a stream writes it.
        let (s, sid, a, one, _rx_a) = claimed_by_a().await;
        let (_, mut rx_b, b) = s.attach_stream(&sid, None).unwrap();
        assert!(s.deliver(observation("obs-1", json!(2))));
        let StreamItem::Observation { id: two, .. } = rx_b.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        assert!(s.claim_stream_item(&sid, b, Some(two)));
        s.finish_stream_item(&sid, a, one, false);
        s.finish_stream_item(&sid, b, two, true);
        drop(rx_b);

        let (backlog_c, _rx_c, c) = s.attach_stream(&sid, Some(two)).unwrap();
        assert_eq!(
            backlog_ids(&backlog_c),
            vec![one],
            "1 is owed, 2 was written"
        );
        assert!(s.claim_stream_item(&sid, c, Some(one)));
        s.finish_stream_item(&sid, c, one, true);

        // Written is not received: the client dropped before reading it
        // and still says 2, so 1 is still owed.
        let (backlog_d, _rx_d, _) = s.attach_stream(&sid, Some(two)).unwrap();
        assert_eq!(backlog_ids(&backlog_d), vec![one], "still owed");
    }

    /// 1 owed after a takeover (see above), and the store at the point the
    /// client resumes from 2: (store, session, one, two).
    async fn owed_one_after_two() -> (Sessions, String, u64, u64) {
        let (s, sid, a, one, _rx_a) = claimed_by_a().await;
        let (_, mut rx_b, b) = s.attach_stream(&sid, None).unwrap();
        assert!(s.deliver(observation("obs-1", json!(2))));
        let StreamItem::Observation { id: two, .. } = rx_b.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        assert!(s.claim_stream_item(&sid, b, Some(two)));
        s.finish_stream_item(&sid, a, one, false);
        s.finish_stream_item(&sid, b, two, true);
        (s, sid, one, two)
    }

    #[tokio::test]
    async fn an_owed_observation_is_settled_when_the_client_reports_it() {
        let (s, sid, one, two) = owed_one_after_two().await;
        let (_, _rx, c) = s.attach_stream(&sid, Some(two)).unwrap();
        assert!(s.claim_stream_item(&sid, c, Some(one)));
        s.finish_stream_item(&sid, c, one, true);
        // The client's Last-Event-ID is 1: it read 1, the last it got.
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(one)).unwrap();
        assert!(!backlog_ids(&backlog).contains(&one), "received");
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(two)).unwrap();
        assert!(!backlog_ids(&backlog).contains(&one), "settled for good");
    }

    #[tokio::test]
    async fn an_owed_observation_is_settled_by_a_later_event_on_its_stream() {
        let (s, sid, one, two) = owed_one_after_two().await;
        let (_, mut rx_c, c) = s.attach_stream(&sid, Some(two)).unwrap();
        assert!(s.claim_stream_item(&sid, c, Some(one)));
        s.finish_stream_item(&sid, c, one, true);
        assert!(s.deliver(observation("obs-1", json!(3))));
        let StreamItem::Observation { id: three, .. } = rx_c.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        assert!(s.claim_stream_item(&sid, c, Some(three)));
        s.finish_stream_item(&sid, c, three, true);
        // 3 came after 1 on the same stream: having 3 means having 1.
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(three)).unwrap();
        assert!(
            backlog_ids(&backlog).is_empty(),
            "{:?}",
            backlog_ids(&backlog)
        );
    }

    #[tokio::test]
    async fn an_event_written_on_two_streams_settles_nothing_it_cannot_vouch_for() {
        // 1 is owed; B wrote 2. C replays from 0 and writes 1, then 2, but
        // the client reads none of C. It resumes from 2, which it got from
        // B: that says nothing about 1, so 1 stays owed.
        let (s, sid, one, two) = owed_one_after_two().await;
        let (backlog_c, _rx_c, c) = s.attach_stream(&sid, Some(0)).unwrap();
        assert_eq!(backlog_ids(&backlog_c), vec![one, two]);
        for id in [one, two] {
            assert!(s.claim_stream_item(&sid, c, Some(id)));
            s.finish_stream_item(&sid, c, id, true);
        }
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(two)).unwrap();
        assert_eq!(backlog_ids(&backlog), vec![one], "2 could have come from B");
    }

    #[tokio::test]
    async fn a_write_still_in_flight_counts_as_a_possible_source() {
        // B claimed 2 and has not reported yet; 1 is owed. C replays and
        // writes 1 and 2, unread. The client got 2 from B's pending write
        // and resumes from 2: 1 stays owed.
        let (s, sid, a, one, _rx_a) = claimed_by_a().await;
        let (_, mut rx_b, b) = s.attach_stream(&sid, None).unwrap();
        assert!(s.deliver(observation("obs-1", json!(2))));
        let StreamItem::Observation { id: two, .. } = rx_b.recv().await.unwrap() else {
            panic!("expected an observation");
        };
        assert!(s.claim_stream_item(&sid, b, Some(two)));
        s.finish_stream_item(&sid, a, one, false);
        let (backlog_c, _rx_c, c) = s.attach_stream(&sid, Some(0)).unwrap();
        assert_eq!(backlog_ids(&backlog_c), vec![one, two]);
        for id in [one, two] {
            assert!(s.claim_stream_item(&sid, c, Some(id)));
            s.finish_stream_item(&sid, c, id, true);
        }
        let (backlog, _rx, _) = s.attach_stream(&sid, Some(two)).unwrap();
        assert_eq!(
            backlog_ids(&backlog),
            vec![one],
            "B's pending 2 may be what it got"
        );
    }
}
