//! Session registry and lifecycle. A `Session` owns one live driver
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

use crate::config::SharedConfig;
use crate::driver::{Driver, RemoteClient, Screen};
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

/// Build the native (drive-this-machine) driver. Only available on desktop
/// OSes; elsewhere the worker requires a guest-executor endpoint.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_driver(
    cfg: &crate::config::WorkerConfig,
    monitor: Option<u32>,
) -> Result<Arc<dyn Driver>, String> {
    // Surface the macOS Screen Recording prompt and fail loudly if capture would
    // be wallpaper-only, so the model is never handed a blank desktop silently.
    if cfg.screen_capture_preflight {
        crate::driver::native::preflight_screen_capture()?;
    }
    Ok(Arc::new(crate::driver::NativeHost::new(
        cfg.max_screenshot_dimension as u32,
        cfg.screenshot_quality as u8,
        monitor,
    )))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_driver(
    _cfg: &crate::config::WorkerConfig,
    _monitor: Option<u32>,
) -> Result<Arc<dyn Driver>, String> {
    Err(
        "native driver (drive this machine) is only available on macOS and Windows; \
         pass an `endpoint` to drive a remote or sandboxed desktop"
            .to_string(),
    )
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
    driver: Arc<dyn Driver>,
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
        driver: Arc<dyn Driver>,
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
            driver,
            screencast_active: AtomicBool::new(false),
            frame_seq: AtomicU64::new(0),
            latest_frame: StdMutex::new(None),
            screencast_task: Mutex::new(None),
            config,
            iii,
        })
    }

    pub fn driver(&self) -> &Arc<dyn Driver> {
        &self.driver
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
    /// active is a no-op. The pump polls the driver screenshot at the
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
            // Wait for the abort to land: a pump mid-push would otherwise
            // write a frame back after the delete below.
            let _ = handle.await;
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
            match self.driver.screenshot().await {
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
                    // Clear what the pump published: a watcher must not keep
                    // showing a frame from before the driver broke.
                    self.delete_frame_stream().await;
                    *self.latest_frame.lock().unwrap_or_else(|p| p.into_inner()) = None;
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
    /// then close the driver. Idempotent.
    async fn shutdown(&self) {
        self.stop_screencast().await;
        if let Err(e) = self.driver.close().await {
            tracing::warn!(session = %self.id, error = %e, "driver close failed");
        }
    }
}

/// A session slot held while its driver connects. The connect can boot a
/// microVM, so the cap has to be claimed before that work starts, not after:
/// dropping the guard releases the slot on every failure path.
struct SlotGuard(Arc<AtomicU64>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
    /// Sessions whose driver is still connecting; counted against the cap.
    connecting: Arc<AtomicU64>,
    counter: AtomicU64,
    config: SharedConfig,
    emitter: Arc<Emitter>,
    iii: Arc<IIIClient>,
}

impl Sessions {
    pub fn new(config: SharedConfig, emitter: Arc<Emitter>, iii: Arc<IIIClient>) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            connecting: Arc::new(AtomicU64::new(0)),
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
        image: Option<String>,
        endpoint: Option<String>,
        os: Option<String>,
        monitor: Option<u32>,
    ) -> Result<Arc<Session>, String> {
        let cfg = self.config.load_full();
        // Claim a slot under the map lock, counting sessions still connecting:
        // the connect below can boot a whole microVM, so two concurrent starts
        // must not both get past the cap and then discover it at insert time.
        let cap_slot = {
            let map = self.map.lock().await;
            let live = map.len() as u64 + self.connecting.load(Ordering::SeqCst);
            if live >= cfg.max_sessions {
                return Err(format!(
                    "session cap reached ({}); stop a session before starting another",
                    cfg.max_sessions
                ));
            }
            self.connecting.fetch_add(1, Ordering::SeqCst);
            SlotGuard(self.connecting.clone())
        };
        // A sandbox image (arg or configured default) boots a fresh desktop in
        // an iii-sandbox; it takes precedence over an endpoint.
        let image = image
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let d = cfg.sandbox_image.trim();
                (!d.is_empty()).then(|| d.to_string())
            });
        let endpoint = endpoint
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty())
            .or_else(|| {
                let d = cfg.default_endpoint.trim();
                (!d.is_empty()).then(|| d.to_string())
            });

        // Driver selection: a sandbox image wins, else a remote endpoint,
        // else the local machine (native driver).
        let (driver, endpoint_used, default_os): (Arc<dyn Driver>, String, String) =
            if let Some(img) = &image {
                let host = crate::driver::IiiSandboxHost::create(
                    self.iii.clone(),
                    img,
                    cfg.sandbox_width as u32,
                    cfg.sandbox_height as u32,
                    cfg.screenshot_quality as u8,
                    cfg.sandbox_network,
                    cfg.sandbox_idle_timeout_secs,
                    cfg.command_timeout_ms,
                )
                .await?;
                let label = format!("sandbox:{}", host.sandbox_id());
                (Arc::new(host), label, "linux".to_string())
            } else if let Some(ep) = &endpoint {
                let client =
                    RemoteClient::connect(ep, cfg.connect_timeout_ms, cfg.command_timeout_ms)
                        .await?;
                let label = client.endpoint().to_string();
                (Arc::new(client), label, cfg.os.clone())
            } else {
                (
                    native_driver(&cfg, monitor)?,
                    "native".to_string(),
                    std::env::consts::OS.to_string(),
                )
            };
        let os = os.filter(|s| !s.is_empty()).unwrap_or(default_os);
        let screen = driver
            .screen_size()
            .await
            .map_err(|e| format!("driver '{endpoint_used}' screen_size failed: {e}"))?;

        let seq = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("c{seq}");
        let session = Session::new(
            id.clone(),
            endpoint_used,
            os,
            screen,
            now_ms(),
            driver,
            self.config.clone(),
            self.iii.clone(),
        );
        // The slot this session claimed becomes the map entry; releasing the
        // guard after the insert keeps the two counts from ever both missing it.
        self.map.lock().await.insert(id.clone(), session.clone());
        drop(cap_slot);
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
        // A live screencast is somebody watching, so it counts as use: the
        // console viewport would otherwise have the desktop stopped under it
        // for the crime of not clicking anything.
        let stale: Vec<String> = {
            let map = self.map.lock().await;
            map.values()
                .filter(|s| s.last_used_ms() < cutoff && !s.screencast_active())
                .map(|s| s.id.clone())
                .collect()
        };
        for id in stale {
            tracing::info!(session = %id, "stopping idle session");
            self.stop(&id, "idle").await;
        }
    }

    /// Reconnect sessions persisted on a previous run. Best-effort: a record
    /// whose driver no longer answers is dropped. Run once at boot.
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
                    // Functions are live before restore finishes, so a session
                    // may already own this id. The live one wins: overwriting
                    // it would strand its driver with nobody left to stop it.
                    let mut map = self.map.lock().await;
                    if map.contains_key(&rec.session_id) {
                        drop(map);
                        tracing::warn!(session = %rec.session_id, "restore: id already live; dropping the reconnected desktop");
                        session.shutdown().await;
                        continue;
                    }
                    map.insert(rec.session_id.clone(), session);
                    drop(map);
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
        let (driver, endpoint_used): (Arc<dyn Driver>, String) = if rec.endpoint == "native" {
            (native_driver(cfg, None)?, "native".to_string())
        } else if let Some(sid) = rec.endpoint.strip_prefix("sandbox:") {
            // Re-attach to a sandbox that outlived the restart; attach re-runs
            // the idempotent display bootstrap and fails if the VM is gone.
            let host = crate::driver::IiiSandboxHost::attach(
                self.iii.clone(),
                sid.to_string(),
                rec.width,
                rec.height,
                cfg.screenshot_quality as u8,
                cfg.command_timeout_ms,
            )
            .await?;
            let label = format!("sandbox:{}", host.sandbox_id());
            (Arc::new(host), label)
        } else {
            let client = RemoteClient::connect(
                &rec.endpoint,
                cfg.connect_timeout_ms,
                cfg.command_timeout_ms,
            )
            .await?;
            let label = client.endpoint().to_string();
            (Arc::new(client), label)
        };
        let screen = driver.screen_size().await?;
        Ok(Session::new(
            rec.session_id.clone(),
            endpoint_used,
            rec.os.clone(),
            screen,
            rec.created_ms,
            driver,
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
        let Value::Array(items) = listed else {
            tracing::warn!(
                scope = STATE_SCOPE,
                "state::list did not answer with an array; restoring nothing"
            );
            return Ok(Vec::new());
        };
        let mut records = Vec::with_capacity(items.len());
        for item in items {
            match serde_json::from_value::<SessionRecord>(item) {
                Ok(rec) => records.push(rec),
                Err(e) => {
                    tracing::warn!(scope = STATE_SCOPE, error = %e, "dropping unreadable session record")
                }
            }
        }
        Ok(records)
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
