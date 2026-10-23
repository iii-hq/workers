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
//! - **A live screen.** The screencast pump keeps exactly one frame per
//!   session (the newest) in memory and fires the worker-owned
//!   `computer::frame-changed` trigger type after each one (see
//!   [`crate::frames`]); viewers read the frame with `computer::frame`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
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
use crate::frames::{FrameChange, FrameChangeKind, FrameNotifier};

/// State scope for persisted session records.
const STATE_SCOPE: &str = "computer_sessions";
/// Bus RPC timeout for the state side-writes.
const SIDE_WRITE_TIMEOUT_MS: u64 = 5_000;
/// How long a screencast runs with no `computer::frame-changed` binding
/// before the pump stops itself (a console window closed without a stop).
#[cfg(not(test))]
const UNWATCHED_STOP: Duration = Duration::from_secs(10);
#[cfg(test)]
const UNWATCHED_STOP: Duration = Duration::from_millis(200);

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

/// Linux: an X server gets the X11 driver; a Wayland desktop (directly or
/// through XWayland) gets a semantic-only session (spec §5.1).
#[cfg(target_os = "linux")]
fn native_driver(
    cfg: &crate::config::WorkerConfig,
    monitor: Option<u32>,
) -> Result<Arc<dyn Driver>, String> {
    use crate::driver::native::{detect, Desktop, WAYLAND};
    Ok(match detect()? {
        Desktop::X11 => Arc::new(crate::driver::NativeHost::new(
            cfg.max_screenshot_dimension as u32,
            cfg.screenshot_quality as u8,
            monitor,
        )),
        Desktop::Wayland => Arc::new(crate::driver::SemanticOnly(WAYLAND.to_string())),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_driver(
    _cfg: &crate::config::WorkerConfig,
    _monitor: Option<u32>,
) -> Result<Arc<dyn Driver>, String> {
    Err(
        "native driver (drive this machine) is only available on macOS, Windows and Linux; \
         pass an `endpoint` to drive a remote or sandboxed desktop"
            .to_string(),
    )
}

/// The accessibility layer for a native session on this OS, counting
/// invalidations on the shared native desktop.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn native_semantic(invalidations: Arc<AtomicU64>) -> Option<crate::semantic::SemanticSession> {
    Some(crate::semantic::SemanticSession::on_desktop(
        crate::semantic::platform::factory(),
        invalidations,
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_semantic(_: Arc<AtomicU64>) -> Option<crate::semantic::SemanticSession> {
    None
}

/// Every native session drives the same mouse, keyboard and windows, so all
/// of them share one act lock and one ref-invalidation counter: input on any
/// of them kills refs on all of them and never interleaves with another's
/// action. A remote or sandboxed desktop gets its own lock and no
/// accessibility layer.
fn desktop_for(endpoint: &str) -> (Arc<Mutex<()>>, Option<crate::semantic::SemanticSession>) {
    static NATIVE_ACT_LOCK: LazyLock<Arc<Mutex<()>>> = LazyLock::new(Default::default);
    static NATIVE_INVALIDATIONS: LazyLock<Arc<AtomicU64>> = LazyLock::new(Default::default);
    if endpoint == "native" {
        (
            NATIVE_ACT_LOCK.clone(),
            native_semantic(NATIVE_INVALIDATIONS.clone()),
        )
    } else {
        (Arc::new(Mutex::new(())), None)
    }
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
    /// Encoded image size (before base64).
    pub bytes: u64,
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
    /// When this worker process created (or restored) this session object;
    /// `frame_seq` restarts with it.
    pub epoch: i64,
    last_used_ms: AtomicI64,
    driver: Arc<dyn Driver>,
    screencast_active: AtomicBool,
    /// Console viewers of the live screen (the page and the corner previews,
    /// in any window): the pump runs while one remains. A mutex, not an
    /// atomic, so a leave and a join never interleave around start/stop.
    screencast_viewers: Mutex<usize>,
    frame_seq: AtomicU64,
    /// The ONLY frame storage: the newest frame, replaced on every capture.
    latest_frame: StdMutex<Option<Arc<LatestFrame>>>,
    screencast_task: Mutex<Option<JoinHandle<()>>>,
    config: SharedConfig,
    frames: Arc<FrameNotifier>,
    /// Accessibility layer: native sessions only.
    pub semantic: Option<crate::semantic::SemanticSession>,
    /// Serializes coordinate and semantic actions on this desktop (one lock
    /// for every native session), so a coordinate action can never land
    /// between a semantic action's checks and its dispatch.
    pub act_lock: Arc<Mutex<()>>,
    /// One `computer::run` at a time on this session.
    // ponytail: per session; two native sessions can still run on one
    // desktop at once (their semantic acts never share pointer or focus).
    pub run_lock: Mutex<()>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: String,
        endpoint: String,
        os: String,
        screen: Screen,
        created_ms: i64,
        driver: Arc<dyn Driver>,
        config: SharedConfig,
        frames: Arc<FrameNotifier>,
        act_lock: Arc<Mutex<()>>,
        semantic: Option<crate::semantic::SemanticSession>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            endpoint,
            os,
            screen,
            created_ms,
            epoch: now_ms(),
            last_used_ms: AtomicI64::new(now_ms()),
            driver,
            screencast_active: AtomicBool::new(false),
            screencast_viewers: Mutex::new(0),
            frame_seq: AtomicU64::new(0),
            latest_frame: StdMutex::new(None),
            screencast_task: Mutex::new(None),
            config,
            frames,
            semantic,
            act_lock,
            run_lock: Mutex::new(()),
        })
    }

    pub fn driver(&self) -> &Arc<dyn Driver> {
        &self.driver
    }

    /// The worker configuration as it is now.
    pub fn config(&self) -> Arc<crate::config::WorkerConfig> {
        self.config.load_full()
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
    /// configured fps and stores each frame in the session's single slot.
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

    /// A viewer joins the live screen (`computer::screencast::start`): the
    /// pump starts with the first, and restarts if a capture error ended it.
    pub async fn acquire_screencast(self: &Arc<Self>) {
        let mut viewers = self.screencast_viewers.lock().await;
        *viewers += 1;
        self.start_screencast().await;
    }

    /// A viewer leaves (`computer::screencast::stop`): the last one out stops
    /// the pump and clears the frame. Never underflows.
    pub async fn release_screencast(&self) {
        let mut viewers = self.screencast_viewers.lock().await;
        if *viewers == 0 {
            return;
        }
        *viewers -= 1;
        if *viewers == 0 {
            self.stop_screencast().await;
        }
    }

    /// Stop the screencast pump and clear the stored frame so a later viewer
    /// does not see a stale image. Idempotent.
    pub async fn stop_screencast(&self) {
        self.screencast_active.store(false, Ordering::SeqCst);
        if let Some(handle) = self.screencast_task.lock().await.take() {
            handle.abort();
            // Wait for the abort to land: a pump mid-capture would otherwise
            // store a frame back after the clear below.
            let _ = handle.await;
        }
        // Release the last frame's buffer immediately; a stopped screencast
        // must not keep a multi-MB image resident.
        self.clear_frame();
    }

    async fn screencast_pump(self: Arc<Self>) {
        let mut unwatched_since: Option<std::time::Instant> = None;
        loop {
            if !self.screencast_active.load(Ordering::Relaxed) {
                break;
            }
            // A window that closed never says stop, but the engine drops its
            // frame-changed binding: with nobody following for a while, stop.
            if self.frames.watching(&self.id) {
                unwatched_since = None;
            } else if unwatched_since
                .get_or_insert_with(std::time::Instant::now)
                .elapsed()
                >= UNWATCHED_STOP
            {
                tracing::info!(session = %self.id, "screencast unwatched; stopping pump");
                self.screencast_active.store(false, Ordering::Relaxed);
                // Never wait here: a leaving viewer may hold the count while
                // it awaits this very task.
                if let Ok(mut viewers) = self.screencast_viewers.try_lock() {
                    *viewers = 0;
                }
                self.clear_frame();
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
                    let bytes = shot.byte_len() as u64;
                    self.store_frame(Arc::new(LatestFrame {
                        data_b64: shot.to_base64(),
                        mime: shot.mime,
                        width: self.screen.width,
                        height: self.screen.height,
                        frame_seq: seq,
                        bytes,
                        timestamp: now_ms(),
                    }));
                }
                Err(e) => {
                    tracing::warn!(session = %self.id, error = %e, "screencast capture failed; stopping pump");
                    self.screencast_active.store(false, Ordering::Relaxed);
                    // A viewer must not keep showing a frame from before the
                    // driver broke.
                    self.clear_frame();
                    break;
                }
            }
        }
    }

    /// Commit a new frame to the session's single slot (the previous frame is
    /// released here), THEN tell `computer::frame-changed` bindings. A viewer
    /// that reads after the notification always finds this frame or a newer
    /// one.
    fn store_frame(&self, frame: Arc<LatestFrame>) {
        let change = FrameChange {
            session_id: self.id.clone(),
            epoch: self.epoch,
            frame_seq: frame.frame_seq,
            change: FrameChangeKind::Updated,
            width: frame.width,
            height: frame.height,
            mime: Some(frame.mime.clone()),
            bytes: Some(frame.bytes),
            timestamp: frame.timestamp,
        };
        *self.latest_frame.lock().unwrap_or_else(|p| p.into_inner()) = Some(frame);
        self.frames.notify(&change);
    }

    /// Drop the stored frame (frees its buffer) and, when there was one, tell
    /// bindings it is gone.
    fn clear_frame(&self) {
        let dropped = self
            .latest_frame
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(frame) = dropped {
            self.frames.notify(&FrameChange {
                session_id: self.id.clone(),
                epoch: self.epoch,
                frame_seq: frame.frame_seq,
                change: FrameChangeKind::Cleared,
                width: frame.width,
                height: frame.height,
                mime: None,
                bytes: None,
                timestamp: now_ms(),
            });
        }
    }

    /// Tear down: stop the screencast pump (clearing the stored frame), then
    /// close the driver. Idempotent.
    async fn shutdown(&self) {
        if let Some(semantic) = &self.semantic {
            semantic.close();
        }
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

/// The `(image, endpoint)` a start drives: the call's own, else the configured
/// `sandbox_image` / `default_endpoint`. A sandbox image boots a fresh desktop
/// in an iii-sandbox and wins over an endpoint; neither drives this machine.
/// `native` pins this machine, configured defaults included.
fn target(
    image: Option<String>,
    endpoint: Option<String>,
    native: bool,
    cfg: &crate::config::WorkerConfig,
) -> Result<(Option<String>, Option<String>), String> {
    let given = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (image, endpoint) = (given(image), given(endpoint));
    if native {
        if image.is_some() || endpoint.is_some() {
            return Err("native drives this machine: drop image and endpoint".to_string());
        }
        return Ok((None, None));
    }
    let configured = |d: &str| given(Some(d.to_string()));
    Ok((
        image.or_else(|| configured(&cfg.sandbox_image)),
        endpoint.or_else(|| configured(&cfg.default_endpoint)),
    ))
}

pub struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
    /// Sessions whose driver is still connecting; counted against the cap.
    connecting: Arc<AtomicU64>,
    counter: AtomicU64,
    config: SharedConfig,
    emitter: Arc<Emitter>,
    frames: Arc<FrameNotifier>,
    iii: Arc<IIIClient>,
}

impl Sessions {
    pub fn new(
        config: SharedConfig,
        emitter: Arc<Emitter>,
        frames: Arc<FrameNotifier>,
        iii: Arc<IIIClient>,
    ) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            connecting: Arc::new(AtomicU64::new(0)),
            counter: AtomicU64::new(0),
            config,
            emitter,
            frames,
            iii,
        })
    }

    /// Connect a new desktop session. Enforces the concurrency cap, resolves
    /// the target ([`target`]), verifies the connection with a `screen_size`
    /// probe, persists the record, and emits `computer::session-started`.
    pub async fn start(
        &self,
        image: Option<String>,
        endpoint: Option<String>,
        os: Option<String>,
        monitor: Option<u32>,
        native: bool,
    ) -> Result<Arc<Session>, String> {
        let cfg = self.config.load_full();
        let (image, endpoint) = target(image, endpoint, native, &cfg)?;
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
                    cfg.sandbox_memory_mb,
                    cfg.sandbox_cpus,
                    cfg.command_timeout_ms,
                )
                .await?;
                let label = format!("sandbox:{}", host.sandbox_id());
                (Arc::new(host), label, "linux".to_string())
            } else if let Some(ep) = &endpoint {
                let client = RemoteClient::connect(
                    ep,
                    cfg.connect_timeout_ms,
                    cfg.command_timeout_ms,
                    cfg.max_screenshot_dimension as u32,
                    cfg.screenshot_quality as u8,
                )
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
        let (act_lock, semantic) = desktop_for(&endpoint_used);
        let session = Session::new(
            id.clone(),
            endpoint_used,
            os,
            screen,
            now_ms(),
            driver,
            self.config.clone(),
            self.frames.clone(),
            act_lock,
            semantic,
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

    /// A caller started driving `id` with `function`: the console's live
    /// preview shows it. Fire-and-forget, like every event here.
    pub async fn activity(&self, id: &str, function: &str) {
        self.emitter
            .emit(
                EventKind::Activity,
                id,
                &crate::events::ActivityEvent {
                    session_id: id.to_string(),
                    function: function.to_string(),
                    timestamp: now_ms(),
                },
            )
            .await;
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
                cfg.max_screenshot_dimension as u32,
                cfg.screenshot_quality as u8,
            )
            .await?;
            let label = client.endpoint().to_string();
            (Arc::new(client), label)
        };
        let screen = driver.screen_size().await?;
        let (act_lock, semantic) = desktop_for(&endpoint_used);
        Ok(Session::new(
            rec.session_id.clone(),
            endpoint_used,
            rec.os.clone(),
            screen,
            rec.created_ms,
            driver,
            self.config.clone(),
            self.frames.clone(),
            act_lock,
            semantic,
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

/// Best-effort side-write onto the bus (state mutation). Returns the error
/// string for the caller to log (durability writes are logged at warn).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{FrameSink, FrameTarget};
    use crate::functions::frame::read_latest;
    use crate::test_support::FakeDesktop;
    use async_trait::async_trait;
    use iii_sdk::trigger::TriggerConfig;
    use std::sync::Weak;

    /// Records every notification it receives, per target function.
    #[derive(Default)]
    struct Viewers(StdMutex<Vec<(String, Value)>>);

    #[async_trait]
    impl FrameSink for Viewers {
        async fn deliver(&self, target: &FrameTarget, payload: Value) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .push((target.function_id.clone(), payload));
            Ok(())
        }
    }

    impl Viewers {
        fn for_fn(&self, function_id: &str) -> Vec<Value> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(f, _)| f == function_id)
                .map(|(_, p)| p.clone())
                .collect()
        }
    }

    fn session(id: &str, notifier: &Arc<FrameNotifier>) -> (Arc<Session>, Arc<FakeDesktop>) {
        crate::test_support::session_on(notifier, id, None)
    }

    fn bind(notifier: &FrameNotifier, id: &str, session_id: &str) {
        notifier
            .add(TriggerConfig {
                id: id.to_string(),
                function_id: format!("viewer::{id}"),
                config: json!({ "session_id": session_id }),
                metadata: None,
                namespace: None,
            })
            .unwrap();
    }

    async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        for _ in 0..400 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn frame_at(seq: u64) -> Arc<LatestFrame> {
        Arc::new(LatestFrame {
            data_b64: "x".repeat(100_000),
            mime: "image/jpeg".to_string(),
            width: 1280,
            height: 800,
            frame_seq: seq,
            bytes: 75_000,
            timestamp: seq as i64,
        })
    }

    #[tokio::test]
    async fn only_the_latest_frame_is_kept() {
        let notifier = FrameNotifier::new(Arc::new(Viewers::default()));
        let (s, _) = session("c1", &notifier);
        let mut previous: Option<Weak<LatestFrame>> = None;
        for seq in 1..=200 {
            let frame = frame_at(seq);
            let weak = Arc::downgrade(&frame);
            s.store_frame(frame);
            // Storing frame N released frame N-1: one resident frame per
            // session, no history.
            if let Some(prev) = previous.take() {
                assert!(prev.upgrade().is_none(), "frame {} still resident", seq - 1);
            }
            previous = Some(weak);
        }
        let out = read_latest(&s, None);
        assert_eq!(out.frame_seq, 200);
        assert_eq!(out.frame.as_deref().map(str::len), Some(100_000));
        assert_eq!(out.epoch, s.epoch);
        // A read with the current cursor carries no image.
        assert!(read_latest(&s, Some(200)).frame.is_none());
    }

    #[tokio::test]
    async fn viewer_reads_the_initial_frame_then_follows_updates_for_its_session_only() {
        let viewers = Arc::new(Viewers::default());
        let notifier = FrameNotifier::new(viewers.clone());
        let (c1, _) = session("c1", &notifier);
        let (c2, _) = session("c2", &notifier);

        // A frame exists before anyone watches: only the initial read shows it.
        c1.start_screencast().await;
        wait_until("first c1 frame", || c1.latest_frame().is_some()).await;
        bind(&notifier, "v1", "c1");
        bind(&notifier, "v2", "c2");
        let initial = read_latest(&c1, None);
        assert!(initial.frame.is_some() && initial.frame_seq >= 1 && initial.active);

        // Then updates arrive as notifications, each followed by a read.
        c2.start_screencast().await;
        let after = initial.frame_seq;
        wait_until("c1 notifications", || {
            viewers
                .for_fn("viewer::v1")
                .iter()
                .any(|p| p["frame_seq"].as_u64().unwrap() > after + 2)
        })
        .await;
        let n = viewers.for_fn("viewer::v1").last().cloned().unwrap();
        assert_eq!(n["session_id"], "c1");
        assert_eq!(n["change"], "updated");
        assert_eq!(n["epoch"], c1.epoch);
        assert_eq!(n["bytes"], 64 * 1024 + 4);
        let fresh = read_latest(&c1, Some(after));
        assert!(fresh.frame.is_some());
        assert!(fresh.frame_seq >= n["frame_seq"].as_u64().unwrap());

        // Filter by session: v1 never hears c2, v2 never hears c1.
        wait_until("c2 notifications", || {
            !viewers.for_fn("viewer::v2").is_empty()
        })
        .await;
        assert!(viewers
            .for_fn("viewer::v1")
            .iter()
            .all(|p| p["session_id"] == "c1"));
        assert!(viewers
            .for_fn("viewer::v2")
            .iter()
            .all(|p| p["session_id"] == "c2"));
        // Within one viewer, revisions only move forward.
        let seqs: Vec<u64> = viewers
            .for_fn("viewer::v1")
            .iter()
            .map(|p| p["frame_seq"].as_u64().unwrap())
            .collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");

        c1.shutdown().await;
        c2.shutdown().await;
    }

    #[tokio::test]
    async fn session_end_clears_the_frame_and_notifies_cleared() {
        let viewers = Arc::new(Viewers::default());
        let notifier = FrameNotifier::new(viewers.clone());
        let (s, desktop) = session("c1", &notifier);
        bind(&notifier, "v", "c1");
        s.start_screencast().await;
        wait_until("a frame", || s.latest_frame().is_some()).await;
        let resident = Arc::downgrade(&s.latest_frame().unwrap());

        s.shutdown().await; // what sessions::stop / idle stop / worker shutdown run
        assert!(!s.screencast_active());
        assert!(s.latest_frame().is_none());
        let out = read_latest(&s, None);
        assert!(out.frame.is_none() && !out.active && out.frame_seq == 0);
        wait_until("cleared notification", || {
            viewers
                .for_fn("viewer::v")
                .last()
                .is_some_and(|p| p["change"] == "cleared")
        })
        .await;
        // The frame buffer is gone once in-flight notifications drained.
        wait_until("frame released", || resident.upgrade().is_none()).await;

        // The pump is really dead: no capture and no notification after stop.
        let shots = desktop.shots.load(Ordering::SeqCst);
        let seen = viewers.for_fn("viewer::v").len();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(desktop.shots.load(Ordering::SeqCst), shots);
        assert_eq!(viewers.for_fn("viewer::v").len(), seen);
        assert!(s.latest_frame().is_none());
    }

    #[tokio::test]
    async fn capture_failure_clears_the_frame() {
        let viewers = Arc::new(Viewers::default());
        let notifier = FrameNotifier::new(viewers.clone());
        let (s, desktop) = session("c1", &notifier);
        bind(&notifier, "v", "c1");
        s.start_screencast().await;
        wait_until("a frame", || s.latest_frame().is_some()).await;
        desktop.fail.store(true, Ordering::SeqCst);
        wait_until("pump stopped", || !s.screencast_active()).await;
        assert!(s.latest_frame().is_none());
        wait_until("cleared notification", || {
            viewers
                .for_fn("viewer::v")
                .last()
                .is_some_and(|p| p["change"] == "cleared")
        })
        .await;
        s.stop_screencast().await;
    }

    #[tokio::test]
    async fn viewers_share_one_screencast_and_the_last_one_out_stops_it() {
        let notifier = FrameNotifier::new(Arc::new(Viewers::default()));
        let (s, _) = session("c1", &notifier);
        bind(&notifier, "page", "c1");
        s.acquire_screencast().await; // the page
        s.acquire_screencast().await; // a corner preview in another window
        wait_until("a frame", || s.latest_frame().is_some()).await;
        s.release_screencast().await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(s.screencast_active(), "the page still watches");
        assert!(s.latest_frame().is_some());
        s.release_screencast().await;
        assert!(!s.screencast_active());
        assert!(s.latest_frame().is_none());
        // A stray leave never underflows into a stop of the next viewer.
        s.release_screencast().await;
        s.acquire_screencast().await;
        assert!(s.screencast_active());
        s.shutdown().await;
    }

    #[tokio::test]
    async fn a_screencast_nobody_follows_stops_itself() {
        let notifier = FrameNotifier::new(Arc::new(Viewers::default()));
        let (s, _) = session("c1", &notifier);
        // A window joined, then closed: no frame-changed binding, no stop.
        s.acquire_screencast().await;
        wait_until("pump stopped", || !s.screencast_active()).await;
        assert!(s.latest_frame().is_none());
        // Its count went with it: the next viewer's leave stops the pump.
        bind(&notifier, "page", "c1");
        s.acquire_screencast().await;
        assert!(s.screencast_active());
        s.release_screencast().await;
        assert!(!s.screencast_active());
    }

    #[tokio::test]
    async fn stopping_without_a_frame_sends_nothing() {
        let viewers = Arc::new(Viewers::default());
        let notifier = FrameNotifier::new(viewers.clone());
        let (s, _) = session("c1", &notifier);
        bind(&notifier, "v", "c1");
        s.stop_screencast().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(viewers.for_fn("viewer::v").is_empty());
    }

    #[tokio::test]
    async fn shutdown_closes_the_semantic_layer() {
        use crate::semantic::{fake::FakeTree, Code, Query, Scope, SemanticSession};
        let tree = Arc::new(FakeTree::new("App", 1));
        let notifier = FrameNotifier::new(Arc::new(Viewers::default()));
        let (s, _) = crate::test_support::session_on(
            &notifier,
            "c9",
            Some(SemanticSession::new(tree.factory())),
        );
        let q = Query {
            scope: Scope::default(),
            selector: None,
            max_nodes: 256,
            max_depth: 6,
        };
        s.semantic
            .as_ref()
            .unwrap()
            .snapshot(q.clone(), true)
            .await
            .unwrap();
        s.shutdown().await;
        let err = s
            .semantic
            .as_ref()
            .unwrap()
            .snapshot(q, true)
            .await
            .unwrap_err();
        assert_eq!(err.code, Code::UnsupportedSession);
    }

    #[test]
    fn native_sessions_share_a_desktop_and_others_do_not() {
        let (a, _) = desktop_for("native");
        let (b, _) = desktop_for("native");
        let (c, semantic) = desktop_for("ws://10.0.0.5:8765");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &c));
        assert!(semantic.is_none());
    }

    #[tokio::test]
    async fn stopping_a_session_never_waits_for_the_shared_desktop() {
        use crate::semantic::{fake::FakeTree, SemanticSession};
        let lock = Arc::new(Mutex::new(()));
        let tree = Arc::new(FakeTree::new("App", 1));
        let (s1, _) = crate::test_support::session_sharing("c1", lock.clone(), None);
        let (s2, _) = crate::test_support::session_sharing(
            "c2",
            lock.clone(),
            Some(SemanticSession::new(tree.factory())),
        );
        let _held = s1.act_lock.lock().await;
        tokio::time::timeout(Duration::from_secs(1), s2.shutdown())
            .await
            .expect("shutdown waited for the shared act lock");
    }

    #[test]
    fn native_pins_this_machine_over_configured_defaults() {
        let cfg = crate::config::WorkerConfig {
            sandbox_image: "anthropic".into(),
            default_endpoint: "guest:7000".into(),
            ..Default::default()
        };
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            target(None, None, false, &cfg),
            Ok((some("anthropic"), some("guest:7000"))),
            "a bare start takes the configured defaults"
        );
        assert_eq!(
            target(some(" cua "), some(" "), false, &cfg),
            Ok((some("cua"), some("guest:7000")))
        );
        assert_eq!(
            target(None, Some(String::new()), true, &cfg),
            Ok((None, None))
        );
        assert!(target(some("cua"), None, true, &cfg).is_err());
        assert!(target(None, some("guest:7000"), true, &cfg).is_err());
    }
}
