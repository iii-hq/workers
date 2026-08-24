//! Session lifecycle: one Chromium process + one page per session, with the
//! CDP event pumps that keep the console/network ring buffers live and fire
//! the `browser::*` custom triggers. The ring buffers are the durable record
//! (`browser::console::read` / `browser::network::read`); the triggers are
//! the live feed the console UI subscribes to.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser as cdp_browser;
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
    Bounds, ConsoleEventPayload, DownloadChangedEvent, Emitter, EventKind, NavigatedEvent,
    PickedElement, PickedEvent, SessionStoppedEvent,
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

/// One entry in a session's navigation history.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct HistoryVisit {
    pub url: String,
    pub title: String,
    pub timestamp: i64,
}

/// How many navigations a session keeps for the history panel.
const HISTORY_CAP: usize = 200;

/// How many downloads a session keeps; the oldest record and its file are
/// dropped past this, like the other per-session buffers.
const DOWNLOADS_CAP: usize = 100;

/// A download Chromium started, tracked by its CDP guid.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct DownloadRecord {
    pub guid: String,
    pub file_name: String,
    pub url: String,
    /// `in_progress`, `completed`, or `canceled`.
    pub state: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub started_ms: i64,
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

/// An in-progress screen recording: the ffmpeg child the screencast pump
/// feeds decoded JPEG frames, plus the bookkeeping `recording::stop` returns.
pub struct Recording {
    pub child: tokio::process::Child,
    /// Bounded queue the screencast pump feeds decoded JPEG frames onto with
    /// `try_send` (dropping frames when full) so the pump never awaits ffmpeg
    /// I/O. Drained by `writer`.
    pub tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// Dedicated task that writes queued frames to ffmpeg's stdin and bumps
    /// `frames` per successful write.
    pub writer: tokio::task::JoinHandle<()>,
    /// Frames actually written to ffmpeg, shared with the writer task.
    pub frames: Arc<AtomicU64>,
    pub path: String,
    pub started_ms: i64,
}

/// Bounded frame queue depth: a few frames of slack so a brief ffmpeg stall
/// doesn't drop everything, small enough that memory stays bounded and stale
/// frames are dropped rather than buffered forever.
const RECORDING_QUEUE_DEPTH: usize = 8;

/// Drain decoded frames onto ffmpeg's stdin, counting successful writes.
/// Exits when the channel closes (recording stopped) or the pipe breaks
/// (ffmpeg gone), shutting stdin so ffmpeg can finalize the file.
async fn recording_writer(
    mut stdin: tokio::process::ChildStdin,
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    frames: Arc<AtomicU64>,
) {
    use tokio::io::AsyncWriteExt;
    while let Some(bytes) = rx.recv().await {
        if stdin.write_all(&bytes).await.is_ok() {
            frames.fetch_add(1, Ordering::Relaxed);
        } else {
            break;
        }
    }
    let _ = stdin.shutdown().await;
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
    /// The live viewport the session renders at. Set at launch, then tracked
    /// to the console pane's size by browser::resize (or overridden by the
    /// device toolbar), so the streamed frame fills the pane with no
    /// letterboxing and click coordinates map 1:1.
    pub viewport_width: AtomicU32,
    pub viewport_height: AtomicU32,
    pub created_ms: i64,
    /// Temp profile dir removed on shutdown. None in persistent
    /// `user_data_dir` mode.
    ephemeral_profile: Option<std::path::PathBuf>,
    pub latest_frame: Mutex<Option<LatestFrame>>,
    /// Number of live consumers of the Chromium screencast: each console
    /// viewer and each recording counts as one. The CDP screencast runs
    /// while this is > 0; it starts on the 0->1 transition and stops on
    /// 1->0, so stopping a recording never cuts off a UI viewer (and vice
    /// versa).
    pub screencast_consumers: std::sync::atomic::AtomicUsize,
    /// Set while a `browser::recording` is capturing; the screencast pump
    /// writes decoded frames into it.
    pub recording: tokio::sync::Mutex<Option<Recording>>,
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
    /// Committed top-document navigations, newest last, for the history
    /// panel and address-bar suggestions. Capped like the other buffers.
    pub history: Mutex<Vec<HistoryVisit>>,
    /// Downloads Chromium started in this session, by CDP guid. None for
    /// attached sessions, where the worker does not own the download policy.
    pub downloads: Mutex<Vec<DownloadRecord>>,
    /// Where `allowAndName` writes files (named by guid); removed on stop.
    pub downloads_dir: Option<std::path::PathBuf>,
    pick_counter: AtomicU64,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Session {
    pub fn touch(&self) {
        self.last_used_ms.store(now_ms() as u64, Ordering::Relaxed);
    }

    pub fn viewport(&self) -> (u32, u32) {
        (
            self.viewport_width.load(Ordering::Relaxed),
            self.viewport_height.load(Ordering::Relaxed),
        )
    }

    pub fn set_viewport(&self, width: u32, height: u32) {
        self.viewport_width.store(width, Ordering::Relaxed);
        self.viewport_height.store(height, Ordering::Relaxed);
    }

    pub fn last_used_ms(&self) -> i64 {
        self.last_used_ms.load(Ordering::Relaxed) as i64
    }

    /// Record a committed navigation. Consecutive visits to the same URL
    /// collapse (a reload is not a new entry); the buffer is capped.
    pub fn record_visit(&self, url: &str, title: &str) {
        if url.is_empty() || url == "about:blank" {
            return;
        }
        let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
        if history.last().is_some_and(|last| last.url == url) {
            if let Some(last) = history.last_mut() {
                if !title.is_empty() {
                    last.title = title.to_string();
                }
                last.timestamp = now_ms();
            }
            return;
        }
        history.push(HistoryVisit {
            url: url.to_string(),
            title: title.to_string(),
            timestamp: now_ms(),
        });
        let len = history.len();
        if len > HISTORY_CAP {
            history.drain(0..len - HISTORY_CAP);
        }
    }

    /// Note a download beginning; later progress updates it by guid.
    pub fn download_begin(&self, guid: &str, file_name: &str, url: &str) {
        let mut downloads = self.downloads.lock().unwrap_or_else(|p| p.into_inner());
        if downloads.iter().any(|d| d.guid == guid) {
            return;
        }
        downloads.push(DownloadRecord {
            guid: guid.to_string(),
            file_name: file_name.to_string(),
            url: url.to_string(),
            state: "in_progress".to_string(),
            received_bytes: 0,
            total_bytes: 0,
            started_ms: now_ms(),
        });
        if downloads.len() > DOWNLOADS_CAP {
            let overflow = downloads.len() - DOWNLOADS_CAP;
            let evicted: Vec<DownloadRecord> = downloads.drain(0..overflow).collect();
            if let Some(dir) = &self.downloads_dir {
                for record in evicted {
                    let _ = std::fs::remove_file(dir.join(&record.guid));
                }
            }
        }
    }

    /// Update a download's progress and state by guid.
    pub fn download_progress(&self, guid: &str, received: u64, total: u64, state: &str) {
        let mut downloads = self.downloads.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(record) = downloads.iter_mut().find(|d| d.guid == guid) {
            record.received_bytes = received;
            record.total_bytes = total.max(received);
            record.state = state.to_string();
        }
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

    /// True while the Chromium screencast is running (at least one consumer:
    /// a console viewer and/or a recording).
    pub fn screencast_on(&self) -> bool {
        self.screencast_consumers.load(Ordering::Relaxed) > 0
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
        // Finalize any recording first: closing the sender ends the writer,
        // which shuts ffmpeg's stdin so it flushes the file; then reap the
        // child so it is not orphaned. Bound the wait so an ffmpeg that
        // ignores its closed stdin cannot block shutdown forever; on timeout,
        // kill it and reap the terminated process.
        if let Some(recording) = self.recording.lock().await.take() {
            let mut child = recording.child;
            drop(recording.tx);
            if tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            let _ = recording.writer.await;
        }
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
                if let Some(dir) = &self.downloads_dir {
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

        // Let the session download files, named by guid into a per-session
        // dir the worker owns, with progress events. Best-effort: a browser
        // that refuses leaves downloads disabled, not the session broken.
        // The dir name carries a nonce so it cannot be squatted in advance,
        // and it is created fresh, owner-only, refusing a pre-existing path.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let downloads_dir = std::env::temp_dir().join(format!(
            "iii-browser-dl-{}-{slot}-{nonce:08x}",
            std::process::id()
        ));
        let mut dir_builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dir_builder.mode(0o700);
        }
        let dir_ok = dir_builder.create(&downloads_dir).is_ok();
        if dir_ok {
            if let Ok(params) = cdp_browser::SetDownloadBehaviorParams::builder()
                .behavior(cdp_browser::SetDownloadBehaviorBehavior::AllowAndName)
                .download_path(downloads_dir.to_string_lossy().to_string())
                .events_enabled(true)
                .build()
            {
                let _ = page.execute(params).await;
            }
        }

        let id = format!("b{slot}");
        let now = now_ms();
        let session = Arc::new(Session {
            id: id.clone(),
            headless,
            kind: SessionKind::Launched,
            read_only,
            viewport_width: AtomicU32::new(cfg.viewport_width),
            viewport_height: AtomicU32::new(cfg.viewport_height),
            created_ms: now,
            ephemeral_profile,
            latest_frame: Mutex::new(None),
            screencast_consumers: std::sync::atomic::AtomicUsize::new(0),
            recording: tokio::sync::Mutex::new(None),
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
            history: Mutex::new(Vec::new()),
            downloads: Mutex::new(Vec::new()),
            downloads_dir: dir_ok.then(|| downloads_dir.clone()),
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
            viewport_width: AtomicU32::new(cfg.viewport_width),
            viewport_height: AtomicU32::new(cfg.viewport_height),
            created_ms: now,
            ephemeral_profile: None,
            latest_frame: Mutex::new(None),
            screencast_consumers: std::sync::atomic::AtomicUsize::new(0),
            recording: tokio::sync::Mutex::new(None),
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
            history: Mutex::new(Vec::new()),
            downloads: Mutex::new(Vec::new()),
            downloads_dir: None,
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
                // Authoritative claim: test-and-set under one lock so two
                // concurrent attaches cannot both adopt the same tab. The
                // per-page `taken` filter above only narrows the candidate
                // set; this is what actually guarantees exclusivity.
                let mut adopted = self
                    .adopted_targets
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if adopted.contains(&target) {
                    return Err(format!(
                        "no adoptable tab matches '{needle}' (open tabs: {})",
                        urls.join(", ")
                    ));
                }
                adopted.insert(target);
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

    /// Start recording a session's viewport to `path`, encoded as `codec`
    /// via ffmpeg reading the screencast JPEG stream on stdin. Ensures
    /// screencast is on (remembering whether we turned it on) so a plain
    /// `recording::start` works without a separate screencast call.
    pub async fn start_recording(
        &self,
        session_id: &str,
        path: &str,
        codec: &str,
    ) -> Result<(), String> {
        let session = self
            .get(session_id)
            .ok_or_else(|| format!("unknown session '{session_id}'"))?;

        // Hold the recording lock across the whole check-and-set so two
        // concurrent starts cannot both spawn ffmpeg and overwrite each
        // other's Recording (leaking a process).
        let mut guard = session.recording.lock().await;
        if guard.is_some() {
            return Err(format!(
                "session '{session_id}' is already recording; stop it first"
            ));
        }

        // Acquire a screencast consumer (starts CDP screencast if we are the
        // first). Released again on any failure below.
        self.acquire_screencast(&session).await?;

        let args = crate::functions::recording::ffmpeg_args(codec, path);
        let spawn = tokio::process::Command::new("ffmpeg")
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match spawn {
            Ok(c) => c,
            Err(e) => {
                self.release_screencast(&session).await;
                return Err(format!(
                    "failed to launch ffmpeg ({e}); install ffmpeg and put it on PATH"
                ));
            }
        };
        let stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                self.release_screencast(&session).await;
                return Err("ffmpeg stdin unavailable".to_string());
            }
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(RECORDING_QUEUE_DEPTH);
        let frames = Arc::new(AtomicU64::new(0));
        let writer = tokio::spawn(recording_writer(stdin, rx, frames.clone()));

        *guard = Some(Recording {
            child,
            tx,
            writer,
            frames,
            path: path.to_string(),
            started_ms: now_ms(),
        });
        Ok(())
    }

    /// Stop a session's recording: close ffmpeg's stdin so it finalizes the
    /// file, wait for it, and turn screencast back off if recording started
    /// it. Returns None when nothing was recording.
    pub async fn stop_recording(&self, session_id: &str) -> Option<(String, i64, u64)> {
        let session = self.get(session_id)?;
        let recording = session.recording.lock().await.take()?;
        let Recording {
            mut child,
            tx,
            writer,
            frames,
            path,
            started_ms,
        } = recording;
        // Closing the sender ends the writer, which drains queued frames and
        // shuts ffmpeg's stdin so ffmpeg finalizes the file.
        drop(tx);
        // Bound the reap so an ffmpeg that ignores its closed stdin cannot
        // hang the caller; on timeout kill and reap the terminated process.
        if tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        // The child is done (or killed), so the writer's pipe is closed and
        // it exits promptly; awaiting it makes the frame count final.
        let _ = writer.await;
        self.release_screencast(&session).await;
        Some((path, now_ms() - started_ms, frames.load(Ordering::Relaxed)))
    }

    /// Register a screencast consumer (a console viewer or a recording).
    /// Starts the Chromium screencast on the 0->1 transition; later
    /// acquisitions just bump the count. On a CDP start failure the count is
    /// rolled back so it stays honest.
    pub async fn acquire_screencast(&self, session: &Arc<Session>) -> Result<(), String> {
        use chromiumoxide::cdp::browser_protocol::page as cdp_page;
        let prev = session
            .screencast_consumers
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if prev > 0 {
            return Ok(());
        }
        let quality = self.config.load().screenshot_quality as i64;
        let params = cdp_page::StartScreencastParams::builder()
            .format(cdp_page::StartScreencastFormat::Jpeg)
            .quality(quality)
            .every_nth_frame(1)
            .build();
        if let Err(e) = session.page.execute(params).await {
            session
                .screencast_consumers
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            return Err(format!("screencast start failed: {e}"));
        }
        Ok(())
    }

    /// Release a screencast consumer. Stops the Chromium screencast on the
    /// 1->0 transition, leaving it running while any other consumer remains
    /// (so stopping a recording never cuts off a UI viewer). Never underflows.
    /// Force a fresh screencast frame (e.g. after a viewport resize) without
    /// racing the acquire/release counter: hold a consumer slot of our own
    /// while re-issuing StartScreencast, so a real viewer's release cannot
    /// hit the 1 -> 0 stop transition mid-restart. With no other viewer the
    /// acquire starts and the release stops the cast; both are the counted
    /// paths, so the counter and the CDP state stay in step.
    pub async fn nudge_screencast(&self, session: &Arc<Session>) {
        if self.acquire_screencast(session).await.is_err() {
            return;
        }
        if session
            .screencast_consumers
            .load(std::sync::atomic::Ordering::Relaxed)
            > 1
        {
            use chromiumoxide::cdp::browser_protocol::page as cdp_page;
            let quality = self.config.load().screenshot_quality as i64;
            let restart = cdp_page::StartScreencastParams::builder()
                .format(cdp_page::StartScreencastFormat::Jpeg)
                .quality(quality)
                .every_nth_frame(1)
                .build();
            let _ = session.page.execute(restart).await;
        }
        self.release_screencast(session).await;
    }

    pub async fn release_screencast(&self, session: &Arc<Session>) {
        use chromiumoxide::cdp::browser_protocol::page as cdp_page;
        let prev = session
            .screencast_consumers
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |n| n.checked_sub(1),
            )
            .unwrap_or(0);
        if prev == 1 {
            let _ = session
                .page
                .execute(cdp_page::StopScreencastParams::default())
                .await;
        }
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
                // A cross-process navigation kills a running screencast with
                // the old renderer; restart it so the stream never blanks.
                sx.nudge_screencast(&s).await;
                // A wedged renderer must not stall the navigation pump on a
                // title read; after a short wait the visit records untitled.
                let title =
                    tokio::time::timeout(std::time::Duration::from_secs(2), s.page.get_title())
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .flatten()
                        .unwrap_or_default();
                s.record_visit(&event.frame.url, &title);
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

    // same-document navigations (hash routes, pushState): single-page apps
    // move this way, so they count as visits and emit the same event
    if let Ok(mut events) = page
        .event_listener::<chromiumoxide::cdp::browser_protocol::page::EventNavigatedWithinDocument>(
        )
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let main_frame = s.page.mainframe().await.ok().flatten();
                if main_frame.is_some_and(|id| id != event.frame_id) {
                    continue;
                }
                let title =
                    tokio::time::timeout(std::time::Duration::from_secs(2), s.page.get_title())
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .flatten()
                        .unwrap_or_default();
                s.record_visit(&event.url, &title);
                sx.emitter
                    .emit(
                        EventKind::Navigated,
                        &s.id,
                        &NavigatedEvent {
                            session_id: s.id.clone(),
                            url: event.url.clone(),
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
                    // Feed the recorder the same capped frame flow. Decode to
                    // JPEG bytes and hand them to the writer task via a bounded
                    // queue with try_send: the pump never awaits ffmpeg I/O
                    // (nor holds a lock across it), and frames are dropped
                    // rather than backing up if the encoder falls behind.
                    {
                        let rec = s.recording.lock().await;
                        if let Some(recording) = rec.as_ref() {
                            let data: &str = stream_frame.frame.data.as_ref();
                            if let Ok(bytes) = STANDARD.decode(data) {
                                let _ = recording.tx.try_send(bytes);
                            }
                        }
                    }
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

    // downloads: guid-named files land in the session's download dir; the
    // begin/progress events feed the downloads panel. Only armed when the
    // worker owns the download policy.
    if session.downloads_dir.is_some() {
        if let Ok(mut events) = page
            .event_listener::<cdp_browser::EventDownloadWillBegin>()
            .await
        {
            let s = session.clone();
            let sx = sessions.clone();
            tasks.push(tokio::spawn(async move {
                while let Some(event) = events.next().await {
                    s.download_begin(&event.guid, &event.suggested_filename, &event.url);
                    emit_download_changed(&sx, &s).await;
                }
            }));
        }
        if let Ok(mut events) = page
            .event_listener::<cdp_browser::EventDownloadProgress>()
            .await
        {
            let s = session.clone();
            let sx = sessions.clone();
            tasks.push(tokio::spawn(async move {
                // Chromium reports progress very often on a fast download;
                // terminal states always emit, in-flight ones at most every
                // 250ms so the bus is not flooded.
                let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
                while let Some(event) = events.next().await {
                    let state = match event.state {
                        cdp_browser::DownloadProgressState::InProgress => "in_progress",
                        cdp_browser::DownloadProgressState::Completed => "completed",
                        cdp_browser::DownloadProgressState::Canceled => "canceled",
                    };
                    s.download_progress(
                        &event.guid,
                        event.received_bytes as u64,
                        event.total_bytes as u64,
                        state,
                    );
                    if state != "in_progress" || last_emit.elapsed().as_millis() >= 250 {
                        last_emit = std::time::Instant::now();
                        emit_download_changed(&sx, &s).await;
                    }
                }
            }));
        }
    }

    tasks
}

async fn emit_download_changed(sessions: &Arc<Sessions>, session: &Arc<Session>) {
    sessions
        .emitter
        .emit(
            EventKind::DownloadChanged,
            &session.id,
            &DownloadChangedEvent {
                session_id: session.id.clone(),
                timestamp: now_ms(),
            },
        )
        .await;
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
