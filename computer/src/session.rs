//! Session registry and lifecycle. A `Session` owns one live backend
//! connection to a desktop plus its screencast pump; `Sessions` is the live
//! table keyed by session id (`c1`, `c2`, ...).
//!
//! Two things here go beyond an ephemeral computer-use client, because iii
//! gives us the primitives for free:
//!
//! - **Durable sessions.** Every session is mirrored into `state` (scope
//!   `computer_sessions`). On boot, [`Sessions::restore`] reconnects them
//!   best-effort, so a worker restart does not lose live desktops.
//! - **A live screen stream.** The screencast pump pushes frames onto the
//!   `computer:frames` stream (`stream::set`, one item per session), so the
//!   console and any number of watchers follow the desktop without polling.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::backend::{Backend, ComputerServerClient, NativeHost, Screen};
use crate::config::SharedConfig;
use crate::events::{Emitter, EventKind, SessionStartedEvent, SessionStoppedEvent};

/// Last-value stream carrying the newest screencast frame per session.
pub const FRAMES_STREAM: &str = "computer:frames";
/// Constant item id: the stream keeps only the newest frame per session group.
const FRAME_ITEM_ID: &str = "frame";
/// State scope for persisted session records.
const STATE_SCOPE: &str = "computer_sessions";
/// Bus RPC timeout for the state/stream side-writes.
const SIDE_WRITE_TIMEOUT_MS: u64 = 5_000;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The newest screencast frame, handed to `computer::frame` without a capture
/// round-trip.
#[derive(Clone)]
pub struct LatestFrame {
    pub data_b64: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub timestamp: i64,
}

/// Persisted session record (state value). Enough to reconnect on boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionRecord {
    session_id: String,
    endpoint: String,
    os: String,
    width: u32,
    height: u32,
    created_ms: i64,
}

pub struct Session {
    pub id: String,
    pub endpoint: String,
    pub os: String,
    pub screen: Screen,
    pub created_ms: i64,
    last_used_ms: AtomicI64,
    backend: Arc<dyn Backend>,
    screencast_active: AtomicBool,
    frame_seq: AtomicU64,
    latest_frame: StdMutex<Option<Arc<LatestFrame>>>,
    screencast_task: Mutex<Option<JoinHandle<()>>>,
    config: SharedConfig,
    iii: Arc<IIIClient>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: String,
        endpoint: String,
        os: String,
        screen: Screen,
        created_ms: i64,
        backend: Arc<dyn Backend>,
        config: SharedConfig,
        iii: Arc<IIIClient>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            endpoint,
            os,
            screen,
            created_ms,
            last_used_ms: AtomicI64::new(now_ms()),
            backend,
            screencast_active: AtomicBool::new(false),
            frame_seq: AtomicU64::new(0),
            latest_frame: StdMutex::new(None),
            screencast_task: Mutex::new(None),
            config,
            iii,
        })
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    pub fn touch(&self) {
        self.last_used_ms.store(now_ms(), Ordering::Relaxed);
    }

    pub fn last_used_ms(&self) -> i64 {
        self.last_used_ms.load(Ordering::Relaxed)
    }

    pub fn screencast_active(&self) -> bool {
        self.screencast_active.load(Ordering::Relaxed)
    }

    pub fn latest_frame(&self) -> Option<Arc<LatestFrame>> {
        self.latest_frame
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn record(&self) -> SessionRecord {
        SessionRecord {
            session_id: self.id.clone(),
            endpoint: self.endpoint.clone(),
            os: self.os.clone(),
            width: self.screen.width,
            height: self.screen.height,
            created_ms: self.created_ms,
        }
    }

    /// Start (or confirm) the screencast pump. Idempotent: a second call while
    /// active is a no-op. The pump polls the backend screenshot at the
    /// configured fps and pushes each frame onto `computer:frames`.
    pub async fn start_screencast(self: &Arc<Self>) {
        if self.screencast_active.swap(true, Ordering::SeqCst) {
            return;
        }
        let session = self.clone();
        let handle = tokio::spawn(async move { session.screencast_pump().await });
        if let Some(old) = self.screencast_task.lock().await.replace(handle) {
            old.abort();
        }
    }

    /// Stop the screencast pump and clear the stream group so a later
    /// subscriber does not see a stale frame. Idempotent.
    pub async fn stop_screencast(&self) {
        self.screencast_active.store(false, Ordering::SeqCst);
        if let Some(handle) = self.screencast_task.lock().await.take() {
            handle.abort();
        }
        self.delete_frame_stream().await;
        // Release the last frame's buffer immediately; a stopped screencast
        // must not keep a multi-MB image resident.
        *self.latest_frame.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    async fn screencast_pump(self: Arc<Self>) {
        loop {
            if !self.screencast_active.load(Ordering::Relaxed) {
                break;
            }
            let interval = self.config.load().screencast_interval_ms().max(1);
            tokio::time::sleep(Duration::from_millis(interval)).await;
            if !self.screencast_active.load(Ordering::Relaxed) {
                break;
            }
            match self.backend.screenshot().await {
                Ok(shot) => {
                    let seq = self.frame_seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let frame = Arc::new(LatestFrame {
                        data_b64: shot.to_base64(),
                        mime: shot.mime,
                        width: self.screen.width,
                        height: self.screen.height,
                        frame_seq: seq,
                        timestamp: now_ms(),
                    });
                    // Push first (borrows), then store the Arc (moves): the
                    // base64 is copied once into the RPC payload and never
                    // deep-cloned into the slot.
                    self.push_frame_stream(&frame).await;
                    *self.latest_frame.lock().unwrap_or_else(|p| p.into_inner()) = Some(frame);
                }
                Err(e) => {
                    tracing::warn!(session = %self.id, error = %e, "screencast capture failed; stopping pump");
                    self.screencast_active.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }
    }

    async fn push_frame_stream(&self, frame: &LatestFrame) {
        let payload = json!({
            "stream_name": FRAMES_STREAM,
            "group_id": self.id,
            "item_id": FRAME_ITEM_ID,
            "data": {
                "data": frame.data_b64,
                "mime": frame.mime,
                "width": frame.width,
                "height": frame.height,
                "frame_seq": frame.frame_seq,
                "timestamp": frame.timestamp,
            }
        });
        if let Err(e) = side_write(&self.iii, "stream::set", payload).await {
            tracing::debug!(session = %self.id, error = %e, "frame stream write failed");
        }
    }

    async fn delete_frame_stream(&self) {
        let payload =
            json!({ "stream_name": FRAMES_STREAM, "group_id": self.id, "item_id": FRAME_ITEM_ID });
        if let Err(e) = side_write(&self.iii, "stream::delete", payload).await {
            tracing::debug!(session = %self.id, error = %e, "frame stream delete failed");
        }
    }

    /// Tear down: stop the screencast pump (clearing its stream and buffer),
    /// then close the backend. Idempotent.
    async fn shutdown(&self) {
        self.stop_screencast().await;
        if let Err(e) = self.backend.close().await {
            tracing::warn!(session = %self.id, error = %e, "backend close failed");
        }
    }
}

pub struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
    counter: AtomicU64,
    config: SharedConfig,
    emitter: Arc<Emitter>,
    iii: Arc<IIIClient>,
}

impl Sessions {
    pub fn new(config: SharedConfig, emitter: Arc<Emitter>, iii: Arc<IIIClient>) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            config,
            emitter,
            iii,
        })
    }

    /// Connect a new desktop session. Enforces the concurrency cap, resolves
    /// the endpoint (arg overrides `default_endpoint`), verifies the
    /// connection with a `screen_size` probe, persists the record, and emits
    /// `computer::session-started`.
    pub async fn start(
        &self,
        endpoint: Option<String>,
        os: Option<String>,
    ) -> Result<Arc<Session>, String> {
        let cfg = self.config.load_full();
        if (self.map.lock().await.len() as u64) >= cfg.max_sessions {
            return Err(format!(
                "session cap reached ({}); stop a session before starting another",
                cfg.max_sessions
            ));
        }
        let endpoint = endpoint
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty())
            .or_else(|| {
                let d = cfg.default_endpoint.trim();
                (!d.is_empty()).then(|| d.to_string())
            });
        let os = os.filter(|s| !s.is_empty()).unwrap_or_else(|| {
            if endpoint.is_none() {
                std::env::consts::OS.to_string()
            } else {
                cfg.os.clone()
            }
        });

        // No endpoint drives the local machine directly (native backend); an
        // endpoint drives a remote or sandboxed desktop over computer-server.
        let (backend, endpoint_used): (Arc<dyn Backend>, String) = match &endpoint {
            Some(ep) => {
                let client = ComputerServerClient::connect(
                    ep,
                    cfg.connect_timeout_ms,
                    cfg.command_timeout_ms,
                )
                .await?;
                let label = client.endpoint().to_string();
                (Arc::new(client), label)
            }
            None => (
                Arc::new(NativeHost::new(
                    cfg.max_screenshot_dimension as u32,
                    cfg.screenshot_quality as u8,
                )),
                "native".to_string(),
            ),
        };
        let screen = backend
            .screen_size()
            .await
            .map_err(|e| format!("backend '{endpoint_used}' screen_size failed: {e}"))?;

        let slot = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("c{slot}");
        let session = Session::new(
            id.clone(),
            endpoint_used,
            os,
            screen,
            now_ms(),
            backend,
            self.config.clone(),
            self.iii.clone(),
        );
        self.map.lock().await.insert(id.clone(), session.clone());
        self.persist(&session).await;
        self.emitter
            .emit(
                EventKind::SessionStarted,
                &id,
                &SessionStartedEvent {
                    session_id: id.clone(),
                    endpoint: session.endpoint.clone(),
                    os: session.os.clone(),
                    screen,
                    timestamp: now_ms(),
                },
            )
            .await;
        Ok(session)
    }

    pub async fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.map.lock().await.get(id).cloned()
    }

    pub async fn list(&self) -> Vec<Arc<Session>> {
        let mut sessions: Vec<Arc<Session>> = self.map.lock().await.values().cloned().collect();
        sessions.sort_by_key(|s| s.created_ms);
        sessions
    }

    /// Stop a session and delete its persisted record. Idempotent: stopping an
    /// unknown or already-stopped id returns `false`.
    pub async fn stop(&self, id: &str, reason: &str) -> bool {
        let removed = self.map.lock().await.remove(id);
        match removed {
            Some(session) => {
                session.shutdown().await;
                self.forget(id).await;
                self.emitter
                    .emit(
                        EventKind::SessionStopped,
                        id,
                        &SessionStoppedEvent {
                            session_id: id.to_string(),
                            reason: reason.to_string(),
                            timestamp: now_ms(),
                        },
                    )
                    .await;
                true
            }
            None => false,
        }
    }

    /// Tear down local resources for every session on worker shutdown. Keeps
    /// the persisted records so [`restore`](Self::restore) can reconnect on
    /// the next boot; does not emit stopped events (subscribers are gone).
    pub async fn stop_all(&self) {
        let sessions: Vec<Arc<Session>> = {
            let mut map = self.map.lock().await;
            map.drain().map(|(_, s)| s).collect()
        };
        for session in sessions {
            session.shutdown().await;
        }
    }

    /// Stop sessions idle longer than `idle_stop_ms` (0 disables).
    pub async fn sweep_idle(&self) {
        let idle_ms = self.config.load().idle_stop_ms;
        if idle_ms == 0 {
            return;
        }
        let cutoff = now_ms() - idle_ms as i64;
        let stale: Vec<String> = {
            let map = self.map.lock().await;
            map.values()
                .filter(|s| s.last_used_ms() < cutoff)
                .map(|s| s.id.clone())
                .collect()
        };
        for id in stale {
            tracing::info!(session = %id, "stopping idle session");
            self.stop(&id, "idle").await;
        }
    }

    /// Reconnect sessions persisted on a previous run. Best-effort: a record
    /// whose backend no longer answers is dropped. Run once at boot.
    pub async fn restore(&self) {
        let records = match self.list_records().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "restore: state::list failed; starting empty");
                return;
            }
        };
        if records.is_empty() {
            return;
        }
        let cfg = self.config.load_full();
        let mut restored = 0u64;
        for rec in records {
            if (self.map.lock().await.len() as u64) >= cfg.max_sessions {
                tracing::warn!("restore: cap reached; leaving remaining records for a later start");
                break;
            }
            match self.reconnect(&rec, &cfg).await {
                Ok(session) => {
                    self.advance_counter_past(&rec.session_id);
                    self.map
                        .lock()
                        .await
                        .insert(rec.session_id.clone(), session);
                    restored += 1;
                }
                Err(e) => {
                    tracing::warn!(session = %rec.session_id, endpoint = %rec.endpoint, error = %e, "restore: reconnect failed; dropping record");
                    self.forget(&rec.session_id).await;
                }
            }
        }
        if restored > 0 {
            tracing::info!(restored, "restored persisted computer sessions");
        }
    }

    async fn reconnect(
        &self,
        rec: &SessionRecord,
        cfg: &crate::config::WorkerConfig,
    ) -> Result<Arc<Session>, String> {
        let (backend, endpoint_used): (Arc<dyn Backend>, String) = if rec.endpoint == "native" {
            (
                Arc::new(NativeHost::new(
                    cfg.max_screenshot_dimension as u32,
                    cfg.screenshot_quality as u8,
                )),
                "native".to_string(),
            )
        } else {
            let client = ComputerServerClient::connect(
                &rec.endpoint,
                cfg.connect_timeout_ms,
                cfg.command_timeout_ms,
            )
            .await?;
            let label = client.endpoint().to_string();
            (Arc::new(client), label)
        };
        let screen = backend.screen_size().await?;
        Ok(Session::new(
            rec.session_id.clone(),
            endpoint_used,
            rec.os.clone(),
            screen,
            rec.created_ms,
            backend,
            self.config.clone(),
            self.iii.clone(),
        ))
    }

    /// Keep the id counter ahead of a restored `cN` so a new session never
    /// reuses a live id.
    fn advance_counter_past(&self, session_id: &str) {
        if let Some(n) = session_id
            .strip_prefix('c')
            .and_then(|n| n.parse::<u64>().ok())
        {
            let mut cur = self.counter.load(Ordering::Relaxed);
            while n > cur {
                match self.counter.compare_exchange_weak(
                    cur,
                    n,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(observed) => cur = observed,
                }
            }
        }
    }

    async fn persist(&self, session: &Session) {
        let payload = json!({ "scope": STATE_SCOPE, "key": session.id, "value": session.record() });
        if let Err(e) = side_write(&self.iii, "state::set", payload).await {
            tracing::warn!(session = %session.id, error = %e, "failed to persist session record; it may be lost on restart");
        }
    }

    async fn forget(&self, id: &str) {
        let payload = json!({ "scope": STATE_SCOPE, "key": id });
        if let Err(e) = side_write(&self.iii, "state::delete", payload).await {
            tracing::warn!(session = %id, error = %e, "failed to delete session record");
        }
    }

    async fn list_records(&self) -> Result<Vec<SessionRecord>, String> {
        let listed = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::list".to_string(),
                payload: json!({ "scope": STATE_SCOPE }),
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
            .map_err(|e| e.to_string())?;
        match listed {
            Value::Array(items) => Ok(items
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect()),
            _ => Ok(Vec::new()),
        }
    }
}

/// Best-effort side-write onto the bus (state/stream mutation). Returns the
/// error string for the caller to log at the level that fits: durability
/// writes (persist/forget) at warn, high-volume stream writes at debug.
async fn side_write(iii: &IIIClient, function_id: &str, payload: Value) -> Result<(), String> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(SIDE_WRITE_TIMEOUT_MS),
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}
