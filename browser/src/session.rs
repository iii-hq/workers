//! Session lifecycle: one Chromium process + one page per session, with the
//! CDP event pumps that keep the console/network ring buffers live and fire
//! the `browser::*` custom triggers. The ring buffers are the durable record
//! (`browser::console::read` / `browser::network::read`); the triggers are
//! the live feed the console UI subscribes to.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::dom;
use chromiumoxide::cdp::browser_protocol::log as cdp_log;
use chromiumoxide::cdp::browser_protocol::network as cdp_network;
use chromiumoxide::cdp::js_protocol::runtime;
use chromiumoxide::Page;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{SharedConfig, WorkerConfig};
use crate::events::{
    Bounds, ConsoleEventPayload, Emitter, EventKind, NavigatedEvent, PickedElement, PickedEvent,
    SessionStoppedEvent,
};

/// Truncation caps for values that end up in ring buffers and event
/// payloads — a page can log megabytes; the model reads a summary.
const MAX_TEXT_LEN: usize = 2_000;
const MAX_ARG_LEN: usize = 300;
const MAX_OUTER_HTML_LEN: usize = 2_000;
const MAX_PICK_TEXT_LEN: usize = 400;
const RECENT_ERRORS_IN_PICK: usize = 3;
/// Minimum wall-clock gap between pushed screencast frames (~15fps ceiling).
const FRAME_MIN_INTERVAL_MS: i64 = 66;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated]", &s[..end])
}

/// Pull `id` and `class` from CDP's flattened `[k0, v0, k1, v1, …]`
/// attribute array, truncating the (often long) class list to `class_cap`.
/// The one place the flattened-attribute convention lives for the
/// element-label callers (`dom::read`, `pick::hint`).
pub fn id_and_classes(attrs: &[String], class_cap: usize) -> (Option<String>, Option<String>) {
    let mut id = None;
    let mut classes = None;
    for pair in attrs.chunks(2) {
        if let [k, v] = pair {
            match k.as_str() {
                "id" => id = Some(v.clone()),
                "class" => classes = Some(truncate(v, class_cap)),
                _ => {}
            }
        }
    }
    (id, classes)
}

/// One captured console/log/exception entry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleEntry {
    /// Monotonic per-session cursor; pass back as `since_seq`.
    pub seq: u64,
    pub timestamp: i64,
    /// `log`, `info`, `warning`, `error`, `debug`, or `exception`.
    pub level: String,
    pub text: String,
    /// `url:line` of the emitting frame, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One open tab reported by `browser::tabs::list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// True when a session already adopted this tab; it cannot be adopted
    /// again until that session stops.
    pub adopted: bool,
}

/// One captured network request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NetworkEntry {
    /// Monotonic per-session cursor; pass back as `since_seq`.
    pub seq: u64,
    pub timestamp: i64,
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub failed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct RingBuffer<T> {
    entries: VecDeque<T>,
    dropped: u64,
    cap: usize,
}

impl<T> RingBuffer<T> {
    fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(cap.min(1_024)),
            dropped: 0,
            cap: cap.max(1),
        }
    }

    fn push(&mut self, entry: T) {
        if self.entries.len() >= self.cap {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(entry);
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The shared read pipeline behind `console::read` and `network::read`:
/// take entries past `since` that `keep` accepts, newest `limit` retained,
/// plus the drop counter and the next-cursor `last_seq`. `seq_of` reads the
/// per-entry cursor.
pub fn read_ring<T: Clone>(
    buf: &Mutex<RingBuffer<T>>,
    since: u64,
    limit: usize,
    keep: impl Fn(&T) -> bool,
    seq_of: impl Fn(&T) -> u64,
) -> (Vec<T>, u64, u64) {
    let guard = buf.lock().unwrap_or_else(|p| p.into_inner());
    let mut entries: Vec<T> = guard
        .iter()
        .filter(|e| seq_of(e) > since && keep(e))
        .cloned()
        .collect();
    let dropped = guard.dropped();
    drop(guard);
    let overflow = entries.len().saturating_sub(limit);
    entries.drain(..overflow);
    let last_seq = entries.last().map(&seq_of).unwrap_or(since);
    (entries, last_seq, dropped)
}

/// Newest screencast frame, replaced in place as Chromium pushes. Holds the
/// CDP event by `Arc` so the push-rate pump only copies a pointer; the one
/// owned copy of the base64 payload happens in the `browser::frame` handler,
/// which runs at poll rate. `seq` is the change cursor `browser::frame`
/// compares against `since_frame`.
pub struct LatestFrame {
    pub frame: Arc<ScreencastFrameEvent>,
    pub seq: u64,
    pub timestamp: i64,
}

type ScreencastFrameEvent = chromiumoxide::cdp::browser_protocol::page::EventScreencastFrame;

impl LatestFrame {
    pub fn width(&self) -> u32 {
        self.frame.metadata.device_width.max(0.0) as u32
    }

    pub fn height(&self) -> u32 {
        self.frame.metadata.device_height.max(0.0) as u32
    }
}

/// Refs kept per session before the safety valve clears the table. Only
/// reachable without a navigation (navigation clears refs), so hitting it
/// means thousands of snapshots against one document; clearing degrades to
/// the fail-closed unknown-ref error, never to a wrong element.
const MAX_REFS: usize = 20_000;

/// How a session came to hold its browser, which decides what shutdown may
/// destroy. A launched session owns its Chromium process outright. An
/// attached session holds a CDP connection into the user's own running
/// browser: shutdown closes at most the one tab the session created
/// (`owns_page`), and an adopted user tab is always released untouched. The
/// browser process itself is never closed or killed in attached mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionKind {
    Launched,
    Attached { owns_page: bool },
}

pub struct Session {
    pub id: String,
    pub headless: bool,
    /// Ownership model deciding what shutdown may destroy; see `SessionKind`.
    pub kind: SessionKind,
    /// Inspection-only session: interaction functions are rejected while
    /// navigation and reads stay available. Immutable for the session's
    /// lifetime.
    pub read_only: bool,
    /// Viewport captured at launch. Config viewport is a session-start
    /// setting (a running session keeps the browser it launched with), so
    /// per-call consumers read these fields, not the live config.
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub created_ms: i64,
    /// Temp profile dir removed on shutdown. None in persistent
    /// `user_data_dir` mode.
    ephemeral_profile: Option<std::path::PathBuf>,
    pub latest_frame: Mutex<Option<LatestFrame>>,
    pub screencast_active: std::sync::atomic::AtomicBool,
    frame_seq: AtomicU64,
    browser: tokio::sync::Mutex<Browser>,
    pub page: Page,
    pub console: Mutex<RingBuffer<ConsoleEntry>>,
    pub network: Mutex<RingBuffer<NetworkEntry>>,
    seq: AtomicU64,
    last_used_ms: AtomicU64,
    /// Snapshot/pick refs (`e1`, `p3`, …) → CDP backend node ids. Cleared on
    /// navigation — backend ids do not survive a document swap. Snapshot
    /// refs accumulate (names are session-monotonic), so a ref from an
    /// earlier snapshot of the same document still resolves to the node it
    /// named instead of colliding with a newer snapshot's numbering.
    pub refs: Mutex<HashMap<String, i64>>,
    /// Session-monotonic counter behind snapshot ref names; never reset
    /// within a session so ref names are unique across snapshots.
    pub ref_counter: AtomicU64,
    /// Bumped on every top-document navigation. Snapshots report it so a
    /// caller can tell which document epoch its refs belong to.
    generation: AtomicU64,
    /// Ref-stripped outline lines of the latest snapshot, the baseline for
    /// `browser::snapshot` diff mode. None before the first snapshot and
    /// after a navigation.
    pub snapshot_keys: Mutex<Option<Vec<String>>>,
    /// Cross-call state for `browser::execute`; lives until the session
    /// stops.
    pub exec_state: Mutex<serde_json::Value>,
    pick_counter: AtomicU64,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Session {
    pub fn touch(&self) {
        self.last_used_ms.store(now_ms() as u64, Ordering::Relaxed);
    }

    pub fn last_used_ms(&self) -> i64 {
        self.last_used_ms.load(Ordering::Relaxed) as i64
    }

    pub fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    pub fn next_pick_ref(&self) -> String {
        format!("p{}", self.pick_counter.fetch_add(1, Ordering::Relaxed) + 1)
    }

    pub fn resolve_ref(&self, r: &str) -> Option<i64> {
        self.refs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(r)
            .copied()
    }

    /// Resolve an element ref to its backend node id, or the one canonical
    /// "unknown ref" error every ref-taking handler shares. The message is
    /// load-bearing for agent self-correction, so it lives in one place.
    pub fn resolve_ref_or_err(&self, r: &str) -> Result<i64, iii_sdk::errors::Error> {
        self.resolve_ref(r).ok_or_else(|| {
            iii_sdk::errors::Error::Handler(format!(
                "unknown ref '{r}' (document generation {}); refs come from browser::snapshot / \
                 browser::dom::read / a pick and die on navigation. Re-snapshot, then use a \
                 fresh ref.",
                self.generation()
            ))
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Merge new snapshot refs into the table. Names are session-monotonic
    /// so this never overwrites an older snapshot's names; the safety valve
    /// clears everything if the table grows past `MAX_REFS` (subsequent old
    /// refs then fail closed as unknown).
    pub fn append_refs(&self, map: HashMap<String, i64>) {
        let mut refs = self.refs.lock().unwrap_or_else(|p| p.into_inner());
        if refs.len().saturating_add(map.len()) > MAX_REFS {
            refs.clear();
        }
        refs.extend(map);
    }

    pub fn add_ref(&self, r: String, backend_node_id: i64) {
        self.refs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(r, backend_node_id);
    }

    /// Navigation invalidation: refs die, the diff baseline dies, and the
    /// document generation advances. `exec_state` survives — it is session
    /// state, not document state.
    fn clear_refs(&self) {
        self.refs.lock().unwrap_or_else(|p| p.into_inner()).clear();
        *self.snapshot_keys.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn push_console(&self, entry: ConsoleEntry) {
        self.console
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(entry);
    }

    fn recent_errors(&self) -> Vec<String> {
        let buf = self.console.lock().unwrap_or_else(|p| p.into_inner());
        let mut errors: Vec<String> = buf
            .iter()
            .filter(|e| e.level == "error" || e.level == "exception")
            .map(|e| e.text.clone())
            .collect();
        let keep = errors.len().saturating_sub(RECENT_ERRORS_IN_PICK);
        errors.drain(..keep);
        errors
    }

    /// Tear down what this session owns and abort the event pumps.
    /// Launched: close (or kill) the whole Chromium process and remove the
    /// ephemeral profile. Attached: close only a tab the session itself
    /// created; an adopted user tab is released untouched, and the user's
    /// browser process is never closed. Idempotent at the caller level:
    /// `Sessions::stop` removes the session from the map first, so a second
    /// stop never reaches here.
    pub async fn shutdown(&self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            task.abort();
        }
        match self.kind {
            SessionKind::Launched => {
                let mut browser = self.browser.lock().await;
                if browser.close().await.is_err() {
                    if let Some(Err(e)) = browser.kill().await {
                        tracing::warn!(session_id = %self.id, error = %e, "browser kill failed");
                    }
                }
                let _ = browser.wait().await;
                if let Some(dir) = &self.ephemeral_profile {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
            SessionKind::Attached { owns_page } => {
                if owns_page {
                    if let Err(e) = self.page.clone().close().await {
                        tracing::debug!(session_id = %self.id, error = %e, "tab close failed");
                    }
                }
            }
        }
    }
}

/// A paused handoff waiting for confirmation: the sender resolves the
/// `browser::handoff` call that parked on it. `session_id` lets a confirm
/// addressed to a session find its pending handoff.
pub struct PendingHandoff {
    pub session_id: String,
    pub confirm: tokio::sync::oneshot::Sender<()>,
}

/// The live session table plus everything a pump task needs.
pub struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
    counter: AtomicU64,
    /// Target ids of user tabs currently adopted by a session. Adoption is
    /// exclusive: a tab in this set cannot be adopted again until its
    /// session stops.
    adopted_targets: Mutex<std::collections::HashSet<String>>,
    /// Handoffs currently paused, keyed by handoff id. A confirm removes and
    /// fires the sender; the parked `browser::handoff` handler then returns.
    pending_handoffs: Mutex<HashMap<String, PendingHandoff>>,
    handoff_counter: AtomicU64,
    pub config: SharedConfig,
    pub emitter: Arc<Emitter>,
    /// For pushing screencast frames onto the `browser:frames` stream, which
    /// the console subscribes to instead of polling.
    pub iii: Arc<iii_sdk::IIIClient>,
}

impl Sessions {
    pub fn new(
        config: SharedConfig,
        emitter: Arc<Emitter>,
        iii: Arc<iii_sdk::IIIClient>,
    ) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            adopted_targets: Mutex::new(std::collections::HashSet::new()),
            pending_handoffs: Mutex::new(HashMap::new()),
            handoff_counter: AtomicU64::new(0),
            config,
            emitter,
            iii,
        })
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.lock().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<Session>> {
        let mut sessions: Vec<_> = self.lock().values().cloned().collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        sessions
    }

    pub fn count(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Launch Chromium, open the page, arm the event pumps, insert into the
    /// table. `url` is already scheme-checked by the caller.
    pub async fn start(
        self: &Arc<Self>,
        url: Option<String>,
        headful_override: Option<bool>,
        read_only: bool,
    ) -> Result<Arc<Session>, String> {
        let cfg = self.config.load_full();
        if self.count() as u64 >= cfg.max_sessions {
            return Err(format!(
                "session limit reached ({}); stop one with browser::sessions::stop",
                cfg.max_sessions
            ));
        }

        let headless = match headful_override {
            Some(headful) => !headful,
            None => cfg.headless,
        };
        let slot = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let (browser_config, ephemeral_profile) = build_browser_config(&cfg, headless, slot)?;

        let (browser, mut handler) = Browser::launch(browser_config)
            .await
            .map_err(|e| format!("failed to launch Chromium: {e}"))?;

        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::debug!(error = %e, "cdp handler event error");
                }
            }
        });

        let page = match browser
            .new_page(url.as_deref().unwrap_or("about:blank"))
            .await
        {
            Ok(p) => p,
            Err(e) => {
                handler_task.abort();
                let mut browser = browser;
                let _ = browser.kill().await;
                return Err(format!("failed to open page: {e}"));
            }
        };

        // Log + Network are not enabled by default; console API events are.
        let _ = page.execute(cdp_log::EnableParams::default()).await;
        let _ = page.execute(cdp_network::EnableParams::default()).await;

        let id = format!("b{slot}");
        let now = now_ms();
        let session = Arc::new(Session {
            id: id.clone(),
            headless,
            kind: SessionKind::Launched,
            read_only,
            viewport_width: cfg.viewport_width,
            viewport_height: cfg.viewport_height,
            created_ms: now,
            ephemeral_profile,
            latest_frame: Mutex::new(None),
            screencast_active: std::sync::atomic::AtomicBool::new(false),
            frame_seq: AtomicU64::new(0),
            browser: tokio::sync::Mutex::new(browser),
            page: page.clone(),
            console: Mutex::new(RingBuffer::new(cfg.console_buffer as usize)),
            network: Mutex::new(RingBuffer::new(cfg.network_buffer as usize)),
            seq: AtomicU64::new(1),
            last_used_ms: AtomicU64::new(now as u64),
            refs: Mutex::new(HashMap::new()),
            ref_counter: AtomicU64::new(0),
            generation: AtomicU64::new(1),
            snapshot_keys: Mutex::new(None),
            exec_state: Mutex::new(serde_json::Value::Object(serde_json::Map::new())),
            pick_counter: AtomicU64::new(0),
            tasks: Mutex::new(vec![handler_task]),
        });

        let pumps = spawn_event_pumps(self.clone(), session.clone()).await;
        session
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(pumps);

        self.lock().insert(id, session.clone());
        Ok(session)
    }

    /// Attach to an already-running browser over CDP and bind a session to
    /// one of its tabs: a fresh tab (`adopt_url_substring` absent, session
    /// owns and later closes it) or an existing user tab matched by URL
    /// substring (adopted exclusively, released untouched on stop). The
    /// caller has already gated on `allow_attach` and scheme-checked `url`.
    pub async fn attach(
        self: &Arc<Self>,
        cdp_url: String,
        url: Option<String>,
        adopt_url_substring: Option<String>,
        read_only: bool,
    ) -> Result<Arc<Session>, String> {
        let cfg = self.config.load_full();
        if self.count() as u64 >= cfg.max_sessions {
            return Err(format!(
                "session limit reached ({}); stop one with browser::sessions::stop",
                cfg.max_sessions
            ));
        }

        let (browser, mut handler) = Browser::connect(cdp_url.clone())
            .await
            .map_err(|e| format!("failed to connect to '{cdp_url}': {e}"))?;
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::debug!(error = %e, "cdp handler event error");
                }
            }
        });

        let outcome: Result<(Page, bool), String> = match &adopt_url_substring {
            Some(needle) => match self.find_adoptable(&browser, needle).await {
                Ok(page) => Ok((page, false)),
                Err(e) => Err(e),
            },
            None => browser
                .new_page(url.as_deref().unwrap_or("about:blank"))
                .await
                .map(|p| (p, true))
                .map_err(|e| format!("failed to open tab: {e}")),
        };
        let (page, owns_page) = match outcome {
            Ok(v) => v,
            Err(e) => {
                handler_task.abort();
                return Err(e);
            }
        };

        let _ = page.execute(cdp_log::EnableParams::default()).await;
        let _ = page.execute(cdp_network::EnableParams::default()).await;

        let slot = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("b{slot}");
        let now = now_ms();
        let session = Arc::new(Session {
            id: id.clone(),
            headless: false,
            kind: SessionKind::Attached { owns_page },
            read_only,
            viewport_width: cfg.viewport_width,
            viewport_height: cfg.viewport_height,
            created_ms: now,
            ephemeral_profile: None,
            latest_frame: Mutex::new(None),
            screencast_active: std::sync::atomic::AtomicBool::new(false),
            frame_seq: AtomicU64::new(0),
            browser: tokio::sync::Mutex::new(browser),
            page: page.clone(),
            console: Mutex::new(RingBuffer::new(cfg.console_buffer as usize)),
            network: Mutex::new(RingBuffer::new(cfg.network_buffer as usize)),
            seq: AtomicU64::new(1),
            last_used_ms: AtomicU64::new(now as u64),
            refs: Mutex::new(HashMap::new()),
            ref_counter: AtomicU64::new(0),
            generation: AtomicU64::new(1),
            snapshot_keys: Mutex::new(None),
            exec_state: Mutex::new(serde_json::Value::Object(serde_json::Map::new())),
            pick_counter: AtomicU64::new(0),
            tasks: Mutex::new(vec![handler_task]),
        });

        let pumps = spawn_event_pumps(self.clone(), session.clone()).await;
        session
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(pumps);

        self.lock().insert(id, session.clone());
        Ok(session)
    }

    /// Connect to a running browser and report its open tabs (url, title,
    /// and whether a session already adopted each). Read-only: opens a
    /// throwaway CDP connection, lists, and drops it without touching any
    /// tab. Used by `browser::tabs::list`.
    pub async fn list_tabs(&self, cdp_url: &str) -> Result<Vec<TabInfo>, String> {
        let (browser, mut handler) = Browser::connect(cdp_url.to_string())
            .await
            .map_err(|e| format!("failed to connect to '{cdp_url}': {e}"))?;
        let pump = tokio::spawn(async move { while handler.next().await.is_some() {} });
        let pages = discovered_pages(&browser).await?;
        let mut tabs = Vec::new();
        for page in pages {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            let title = page.get_title().await.ok().flatten();
            let target = page.target_id().as_ref().to_string();
            let adopted = self
                .adopted_targets
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains(&target);
            tabs.push(TabInfo {
                url,
                title,
                adopted,
            });
        }
        pump.abort();
        Ok(tabs)
    }

    /// Resolve a user tab by URL substring for adoption. Exactly one match
    /// is required; zero or several fail closed with the candidate list so
    /// the caller can narrow the substring. A tab already adopted by another
    /// session is excluded up front.
    async fn find_adoptable(&self, browser: &Browser, needle: &str) -> Result<Page, String> {
        let pages = discovered_pages(browser).await?;
        let mut candidates = Vec::new();
        let mut urls = Vec::new();
        for page in pages {
            let url = page.url().await.ok().flatten().unwrap_or_default();
            let target = page.target_id().as_ref().to_string();
            let taken = self
                .adopted_targets
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .contains(&target);
            if url.contains(needle) && !taken {
                candidates.push((page, target));
            }
            urls.push(url);
        }
        match candidates.len() {
            1 => {
                let (page, target) = candidates.into_iter().next().expect("one candidate");
                self.adopted_targets
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(target);
                Ok(page)
            }
            0 => Err(format!(
                "no adoptable tab matches '{needle}' (open tabs: {})",
                urls.join(", ")
            )),
            n => Err(format!(
                "'{needle}' matches {n} tabs; use a longer substring (open tabs: {})",
                urls.join(", ")
            )),
        }
    }

    /// Remove + shut down. Returns whether the session was running —
    /// stopping an unknown id succeeds (delete semantics: the caller wants
    /// it gone, and it is).
    pub async fn stop(&self, id: &str, reason: &str) -> bool {
        let session = self.lock().remove(id);
        match session {
            Some(session) => {
                if let SessionKind::Attached { owns_page: false } = session.kind {
                    let target = session.page.target_id().as_ref().to_string();
                    self.adopted_targets
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&target);
                }
                // Drop any handoff parked on this session so its call
                // unblocks (its receiver errors) instead of waiting for the
                // full timeout against a dead page.
                {
                    let mut pending = self
                        .pending_handoffs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    pending.retain(|_, h| h.session_id != id);
                }
                session.shutdown().await;
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

    /// Register a paused handoff and return its id plus the receiver the
    /// caller awaits. A confirm addressed to the same session (or this id)
    /// fires the sender.
    pub fn register_handoff(
        &self,
        session_id: &str,
    ) -> (String, tokio::sync::oneshot::Receiver<()>) {
        let id = format!(
            "h{}",
            self.handoff_counter.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_handoffs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id.clone(),
                PendingHandoff {
                    session_id: session_id.to_string(),
                    confirm: tx,
                },
            );
        (id, rx)
    }

    /// Drop a pending handoff without confirming (timeout or session stop).
    pub fn drop_handoff(&self, handoff_id: &str) {
        self.pending_handoffs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(handoff_id);
    }

    /// Confirm a paused handoff: by exact id, or the one pending handoff for
    /// a session. Returns the resolved handoff id, or None when nothing
    /// matched (already confirmed, timed out, or wrong session).
    pub fn confirm_handoff(
        &self,
        handoff_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<String> {
        let mut pending = self
            .pending_handoffs
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let key = match (handoff_id, session_id) {
            (Some(id), _) => pending.contains_key(id).then(|| id.to_string()),
            (None, Some(sid)) => pending
                .iter()
                .find(|(_, h)| h.session_id == sid)
                .map(|(k, _)| k.clone()),
            (None, None) => None,
        }?;
        let handoff = pending.remove(&key)?;
        // A closed receiver (parked call already gone) is fine: the confirm
        // still succeeds in removing the stale entry.
        let _ = handoff.confirm.send(());
        Some(key)
    }

    /// Stop sessions idle beyond the configured threshold. Called from the
    /// sweep task in `main`.
    pub async fn sweep_idle(&self) {
        let idle_stop_ms = self.config.load().idle_stop_ms;
        if idle_stop_ms == 0 {
            return;
        }
        let cutoff = now_ms() - idle_stop_ms as i64;
        let idle: Vec<String> = self
            .list()
            .into_iter()
            .filter(|s| s.last_used_ms() < cutoff)
            .map(|s| s.id.clone())
            .collect();
        for id in idle {
            tracing::info!(session_id = %id, "stopping idle session");
            self.stop(&id, "idle").await;
        }
    }

    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.lock().keys().cloned().collect();
        for id in ids {
            self.stop(&id, "stopped").await;
        }
    }
}

/// `Browser::pages()` reads the handler's tracked targets, which populate
/// asynchronously from `Target.targetCreated` events fired after connect.
/// Called immediately, it races those events and returns empty against a
/// browser that has tabs. Poll briefly until a page target appears (or the
/// budget expires — a browser with genuinely no page tabs returns empty).
async fn discovered_pages(browser: &Browser) -> Result<Vec<Page>, String> {
    const ATTEMPTS: u32 = 20;
    for attempt in 0..ATTEMPTS {
        let pages = browser
            .pages()
            .await
            .map_err(|e| format!("failed to list tabs: {e}"))?;
        if !pages.is_empty() || attempt == ATTEMPTS - 1 {
            return Ok(pages);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(Vec::new())
}

/// Every session gets its own profile dir: Chromium holds a process
/// singleton per profile, so two sessions sharing one dir cannot coexist
/// (and a killed session's stale SingletonLock would block the next
/// launch). Ephemeral mode mints a temp dir removed on shutdown;
/// persistent mode maps session slot N to `<user_data_dir>/session-N`, so
/// the first session after every worker boot reuses the same cookies.
fn build_browser_config(
    cfg: &WorkerConfig,
    headless: bool,
    slot: u64,
) -> Result<(BrowserConfig, Option<std::path::PathBuf>), String> {
    let (profile_dir, ephemeral) = if cfg.user_data_dir.is_empty() {
        let dir = std::env::temp_dir().join(format!("iii-browser-{}-{slot}", std::process::id()));
        (dir.clone(), Some(dir))
    } else {
        (
            std::path::PathBuf::from(&cfg.user_data_dir).join(format!("session-{slot}")),
            None,
        )
    };

    let mut builder = BrowserConfig::builder()
        .window_size(cfg.viewport_width, cfg.viewport_height)
        .viewport(chromiumoxide::handler::viewport::Viewport {
            width: cfg.viewport_width,
            height: cfg.viewport_height,
            ..Default::default()
        })
        .user_data_dir(&profile_dir);
    builder = if headless {
        builder.new_headless_mode()
    } else {
        builder.with_head()
    };
    if !cfg.executable.is_empty() {
        builder = builder.chrome_executable(&cfg.executable);
    }
    Ok((builder.build()?, ephemeral))
}

/// Arm the per-session CDP event listeners. Each pump owns one event stream,
/// pushes into the ring buffer, and (console + pick) forwards to trigger
/// subscribers.
async fn spawn_event_pumps(
    sessions: Arc<Sessions>,
    session: Arc<Session>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::new();
    let page = &session.page;

    // console.* calls
    if let Ok(mut events) = page
        .event_listener::<runtime::EventConsoleApiCalled>()
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let text = event
                    .args
                    .iter()
                    .map(preview_remote_object)
                    .collect::<Vec<_>>()
                    .join(" ");
                let entry = ConsoleEntry {
                    seq: s.next_seq(),
                    timestamp: now_ms(),
                    level: console_level(&event.r#type),
                    text: truncate(&text, MAX_TEXT_LEN),
                    source: None,
                };
                push_and_emit(&sx, &s, entry).await;
            }
        }));
    }

    // uncaught exceptions
    if let Ok(mut events) = page.event_listener::<runtime::EventExceptionThrown>().await {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let details = &event.exception_details;
                let description = details
                    .exception
                    .as_ref()
                    .and_then(|e| e.description.clone())
                    .unwrap_or_else(|| details.text.clone());
                let source = details
                    .url
                    .as_ref()
                    .map(|u| format!("{}:{}", u, details.line_number));
                let entry = ConsoleEntry {
                    seq: s.next_seq(),
                    timestamp: now_ms(),
                    level: "exception".to_string(),
                    text: truncate(&description, MAX_TEXT_LEN),
                    source,
                };
                push_and_emit(&sx, &s, entry).await;
            }
        }));
    }

    // browser-level log entries (network errors, security, deprecations)
    if let Ok(mut events) = page.event_listener::<cdp_log::EventEntryAdded>().await {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let entry = &event.entry;
                let source = entry
                    .url
                    .as_ref()
                    .map(|u| format!("{}:{}", u, entry.line_number.unwrap_or(0)));
                let entry = ConsoleEntry {
                    seq: s.next_seq(),
                    timestamp: now_ms(),
                    level: log_level(&entry.level),
                    text: truncate(&entry.text, MAX_TEXT_LEN),
                    source,
                };
                push_and_emit(&sx, &s, entry).await;
            }
        }));
    }

    // network: request → response/failure, correlated by request id
    let pending: Arc<Mutex<HashMap<String, (String, String)>>> =
        Arc::new(Mutex::new(HashMap::new()));

    if let Ok(mut events) = page
        .event_listener::<cdp_network::EventRequestWillBeSent>()
        .await
    {
        let pending = pending.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let mut pending = pending.lock().unwrap_or_else(|p| p.into_inner());
                if pending.len() >= 2_048 {
                    pending.clear();
                }
                pending.insert(
                    event.request_id.inner().to_string(),
                    (event.request.method.clone(), event.request.url.clone()),
                );
            }
        }));
    }

    if let Ok(mut events) = page
        .event_listener::<cdp_network::EventResponseReceived>()
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        let pending = pending.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let key = event.request_id.inner().to_string();
                let method = {
                    let mut pending = pending.lock().unwrap_or_else(|p| p.into_inner());
                    pending.remove(&key).map(|(m, _)| m)
                };
                let entry = NetworkEntry {
                    seq: s.next_seq(),
                    timestamp: now_ms(),
                    method: method.unwrap_or_else(|| "GET".to_string()),
                    url: truncate(&event.response.url, MAX_TEXT_LEN),
                    status: Some(event.response.status),
                    mime_type: Some(event.response.mime_type.clone()),
                    failed: event.response.status >= 400,
                    error: None,
                };
                push_network_and_emit(&sx, &s, entry).await;
            }
        }));
    }

    if let Ok(mut events) = page
        .event_listener::<cdp_network::EventLoadingFailed>()
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        let pending = pending.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let key = event.request_id.inner().to_string();
                let (method, url) = {
                    let mut pending = pending.lock().unwrap_or_else(|p| p.into_inner());
                    pending
                        .remove(&key)
                        .unwrap_or_else(|| ("GET".to_string(), String::new()))
                };
                if event.canceled.unwrap_or(false) {
                    continue;
                }
                let entry = NetworkEntry {
                    seq: s.next_seq(),
                    timestamp: now_ms(),
                    method,
                    url: truncate(&url, MAX_TEXT_LEN),
                    status: None,
                    mime_type: None,
                    failed: true,
                    error: Some(event.error_text.clone()),
                };
                push_network_and_emit(&sx, &s, entry).await;
            }
        }));
    }

    // committed navigations: emit + drop stale element refs
    if let Ok(mut events) = page
        .event_listener::<chromiumoxide::cdp::browser_protocol::page::EventFrameNavigated>()
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if event.frame.parent_id.is_some() {
                    continue; // subframes don't invalidate top-document refs
                }
                s.clear_refs();
                sx.emitter
                    .emit(
                        EventKind::Navigated,
                        &s.id,
                        &NavigatedEvent {
                            session_id: s.id.clone(),
                            url: event.frame.url.clone(),
                            timestamp: now_ms(),
                        },
                    )
                    .await;
            }
        }));
    }

    // screencast frames: keep only the newest, ack every frame so Chromium
    // keeps pushing
    if let Ok(mut events) = page
        .event_listener::<chromiumoxide::cdp::browser_protocol::page::EventScreencastFrame>()
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            let mut last_push_ms = 0i64;
            while let Some(event) = events.next().await {
                let ack_id = event.session_id;
                let now = now_ms();
                // Wall-clock rate cap: an animated page can produce ~60
                // compositor frames a second, but the console only needs a
                // smooth ~15fps. Drop frames that arrive inside the interval
                // (still ack them so Chromium keeps sending); a static page
                // that produces one frame is never starved.
                if now - last_push_ms >= FRAME_MIN_INTERVAL_MS {
                    last_push_ms = now;
                    let seq = s.frame_seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let stream_frame = LatestFrame {
                        frame: event,
                        seq,
                        timestamp: now,
                    };
                    // Push onto the stream (the console subscribes to it); the
                    // in-memory slot stays as the seed for a fresh subscriber.
                    push_frame_stream(&sx.iii, &s.id, &stream_frame).await;
                    let mut slot = s.latest_frame.lock().unwrap_or_else(|p| p.into_inner());
                    *slot = Some(stream_frame);
                }
                let ack = chromiumoxide::cdp::browser_protocol::page::ScreencastFrameAckParams::new(
                    ack_id,
                );
                if let Err(e) = s.page.execute(ack).await {
                    tracing::debug!(session_id = %s.id, error = %e, "screencast ack failed");
                }
            }
        }));
    }

    tasks
}

async fn push_and_emit(sessions: &Arc<Sessions>, session: &Arc<Session>, entry: ConsoleEntry) {
    session.push_console(entry.clone());
    sessions
        .emitter
        .emit(
            EventKind::ConsoleEvent,
            &session.id,
            &ConsoleEventPayload {
                session_id: session.id.clone(),
                entry,
            },
        )
        .await;
}

async fn push_network_and_emit(
    sessions: &Arc<Sessions>,
    session: &Arc<Session>,
    entry: NetworkEntry,
) {
    session
        .network
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(entry.clone());
    sessions
        .emitter
        .emit(
            EventKind::NetworkEvent,
            &session.id,
            &crate::events::NetworkEventPayload {
                session_id: session.id.clone(),
                entry,
            },
        )
        .await;
}

/// The stream the console subscribes to for live viewport frames. Each frame
/// overwrites one item per session (constant item_id, group = session id), so
/// the stream is last-value: subscribers get every new frame pushed, and the
/// stream never retains more than one item per session.
pub const FRAMES_STREAM: &str = "browser:frames";

async fn push_frame_stream(iii: &iii_sdk::IIIClient, session_id: &str, frame: &LatestFrame) {
    let data: &str = frame.frame.data.as_ref();
    let res = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "stream::set".to_string(),
            payload: serde_json::json!({
                "stream_name": FRAMES_STREAM,
                "group_id": session_id,
                "item_id": "frame",
                "data": {
                    "frame": data,
                    "width": frame.width(),
                    "height": frame.height(),
                    "frame_seq": frame.seq,
                    "timestamp": frame.timestamp,
                },
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    if let Err(e) = res {
        tracing::debug!(session_id, error = %e, "frame stream push failed");
    }
}

/// Resolve a pick from viewport coordinates: hit-test the exact point with
/// `DOM.getNodeForLocation` (piercing shadow roots and iframes), then run the
/// shared resolution. This is the console's pick path, so the element picked
/// is exactly the one under the cursor the hover highlight drew, with no
/// dependency on inspect-mode intercepting a synthesized click (which is
/// unreliable in headless).
pub async fn resolve_pick_at(
    sessions: &Arc<Sessions>,
    session: &Arc<Session>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let params = dom::GetNodeForLocationParams::builder()
        .x(x as i64)
        .y(y as i64)
        .include_user_agent_shadow_dom(true)
        .build()
        .map_err(|e| format!("GetNodeForLocation params: {e}"))?;
    let located = session
        .page
        .execute(params)
        .await
        .map_err(|e| format!("DOM.getNodeForLocation: {e}"))?;
    handle_pick(sessions, session, *located.backend_node_id.inner()).await
}

/// Resolve the picked node into a `PickedElement`, register a ref for it,
/// and emit `browser::picked`.
async fn handle_pick(
    sessions: &Arc<Sessions>,
    session: &Arc<Session>,
    backend_node_id: i64,
) -> Result<(), String> {
    let page = &session.page;
    let node_id = dom::BackendNodeId::new(backend_node_id);

    let describe = page
        .execute(
            dom::DescribeNodeParams::builder()
                .backend_node_id(node_id)
                .build(),
        )
        .await
        .map_err(|e| format!("DOM.describeNode: {e}"))?;
    let node = &describe.node;
    let tag = node.node_name.to_lowercase();
    let mut attributes = HashMap::new();
    if let Some(attrs) = &node.attributes {
        for pair in attrs.chunks(2) {
            if let [k, v] = pair {
                attributes.insert(k.clone(), truncate(v, MAX_ARG_LEN));
            }
        }
    }

    let outer_html = page
        .execute(
            dom::GetOuterHtmlParams::builder()
                .backend_node_id(node_id)
                .build(),
        )
        .await
        .map(|r| truncate(&r.outer_html, MAX_OUTER_HTML_LEN))
        .unwrap_or_default();

    let bounds = page
        .execute(
            dom::GetBoxModelParams::builder()
                .backend_node_id(node_id)
                .build(),
        )
        .await
        .ok()
        .map(|r| quad_bounds(&r.model.content))
        .unwrap_or(Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });

    let text = element_inner_text(&outer_html);

    let url = page.url().await.ok().flatten().unwrap_or_default();

    let r#ref = session.next_pick_ref();
    session.add_ref(r#ref.clone(), backend_node_id);

    let element = PickedElement {
        r#ref,
        tag,
        attributes,
        outer_html,
        text,
        bounds,
        url,
        console_recent: session.recent_errors(),
    };

    sessions
        .emitter
        .emit(
            EventKind::Picked,
            &session.id,
            &PickedEvent {
                session_id: session.id.clone(),
                element,
                timestamp: now_ms(),
            },
        )
        .await;
    session.touch();
    Ok(())
}

/// Bounds from a CDP content quad `[x1,y1,x2,y2,x3,y3,x4,y4]`.
pub fn quad_bounds(quad: &dom::Quad) -> Bounds {
    let points = quad.inner();
    let xs: Vec<f64> = points.iter().step_by(2).copied().collect();
    let ys: Vec<f64> = points.iter().skip(1).step_by(2).copied().collect();
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !min_x.is_finite() || !min_y.is_finite() {
        return Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    }
    Bounds {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

/// Cheap innerText approximation from outer HTML — good enough for a chat
/// chip; the model can `browser::evaluate` for the real thing.
fn element_inner_text(outer_html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in outer_html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    truncate(
        text.split_whitespace().collect::<Vec<_>>().join(" ").trim(),
        MAX_PICK_TEXT_LEN,
    )
}

fn preview_remote_object(obj: &runtime::RemoteObject) -> String {
    let raw = if let Some(v) = &obj.value {
        match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    } else if let Some(d) = &obj.description {
        d.clone()
    } else if let Some(u) = &obj.unserializable_value {
        u.inner().to_string()
    } else {
        format!("[{}]", obj.r#type.as_ref())
    };
    truncate(&raw, MAX_ARG_LEN)
}

fn console_level(t: &runtime::ConsoleApiCalledType) -> String {
    match t {
        runtime::ConsoleApiCalledType::Log => "log",
        runtime::ConsoleApiCalledType::Debug => "debug",
        runtime::ConsoleApiCalledType::Info => "info",
        runtime::ConsoleApiCalledType::Error => "error",
        runtime::ConsoleApiCalledType::Warning => "warning",
        _ => "log",
    }
    .to_string()
}

fn log_level(l: &cdp_log::LogEntryLevel) -> String {
    match l {
        cdp_log::LogEntryLevel::Verbose => "debug",
        cdp_log::LogEntryLevel::Info => "info",
        cdp_log::LogEntryLevel::Warning => "warning",
        cdp_log::LogEntryLevel::Error => "error",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_drops_oldest_and_counts() {
        let mut buf = RingBuffer::new(2);
        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.dropped(), 1);
        assert_eq!(buf.iter().copied().collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "héllo wörld";
        let t = truncate(s, 3);
        assert!(t.starts_with("hé") || t.starts_with("h"));
        assert!(t.ends_with("[truncated]"));
        assert_eq!(truncate("short", 100), "short");
    }

    #[test]
    fn inner_text_strips_tags() {
        let html = "<button class=\"x\"> Place <b>order</b> </button>";
        assert_eq!(element_inner_text(html), "Place order");
    }

    #[test]
    fn quad_bounds_from_rect() {
        let quad = dom::Quad::new(vec![10.0, 20.0, 110.0, 20.0, 110.0, 60.0, 10.0, 60.0]);
        let b = quad_bounds(&quad);
        assert_eq!(b.x, 10.0);
        assert_eq!(b.y, 20.0);
        assert_eq!(b.width, 100.0);
        assert_eq!(b.height, 40.0);
    }
}
