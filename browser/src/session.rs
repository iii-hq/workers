//! The browser model: one shared Chromium process (the browser) with one
//! page per tab, the way a real browser works. A `Tab` is the durable
//! record — url, title, history, back/forward stack — that survives the page
//! being closed and, for regular tabs, the worker restarting (`tabs.json`
//! under `data_dir`). A `Session` is a tab with its page open: the CDP event
//! pumps that keep the console/network ring buffers live and fire the
//! `browser::*` custom triggers hang off it. Selecting or calling into a
//! sleeping tab wakes it: the page is opened again at the remembered url.
//! Incognito tabs live in their own throwaway browser context and never
//! touch `data_dir`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser as cdp_browser;
use chromiumoxide::cdp::browser_protocol::dom;
use chromiumoxide::cdp::browser_protocol::fetch as cdp_fetch;
use chromiumoxide::cdp::browser_protocol::log as cdp_log;
use chromiumoxide::cdp::browser_protocol::network as cdp_network;
use chromiumoxide::cdp::browser_protocol::page as cdp_page;
use chromiumoxide::cdp::browser_protocol::target as cdp_target;
use chromiumoxide::cdp::js_protocol::runtime;
use chromiumoxide::Page;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{
    origin_label, origin_policy_config_key_for, origin_policy_for, SharedConfig, WorkerConfig,
};
use crate::events::{
    Bounds, ConsoleEventPayload, DownloadChangedEvent, Emitter, EventKind, FrameEventPayload,
    NavigatedEvent, PickedElement, PickedEvent, SessionStoppedEvent, SessionUpdatedEvent,
};

/// Truncation caps for values that end up in ring buffers and event
/// payloads — a page can log megabytes; the model reads a summary.
const MAX_TEXT_LEN: usize = 2_000;
const MAX_ARG_LEN: usize = 300;
const MAX_OUTER_HTML_LEN: usize = 2_000;
const MAX_PICK_TEXT_LEN: usize = 400;
const RECENT_ERRORS_IN_PICK: usize = 3;
/// Minimum wall-clock gap between pushed screencast frames (~30fps ceiling).
/// Chromium emits up to 60; the console cannot show more than this, and each
/// frame crosses the bus as base64 JSON.
const FRAME_MIN_INTERVAL_MS: i64 = 33;
/// Chromium's error text for a committed HTTP error status (a 4xx/5xx with
/// an empty body). The tab shows the response like any browser does, so it is
/// reported rather than treated as a failed navigation.
pub const HTTP_ERROR_STATUS: &str = "net::ERR_HTTP_RESPONSE_CODE_FAILURE";

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn temp_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn create_private_temp_dir(prefix: &str, owner: &str, nonce: u128) -> std::io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{owner}-{nonce:032x}",
        std::process::id()
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&path)?;
    Ok(path)
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

/// One entry in a tab's navigation history.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HistoryVisit {
    pub url: String,
    pub title: String,
    pub timestamp: i64,
}

/// How many navigations a tab keeps for the history panel.
const HISTORY_CAP: usize = 200;
/// How many entries the back/forward stack keeps.
const NAV_CAP: usize = 100;

/// How many downloads a tab keeps; the oldest record and its file are
/// dropped past this, like the other per-session buffers.
const DOWNLOADS_CAP: usize = 100;

/// How many upload staging directories a session retains while the page may
/// still read their files.
const UPLOAD_DIRS_CAP: usize = 16;

fn remove_oldest_upload_dir(upload_dirs: &mut Vec<PathBuf>) -> Result<(), String> {
    let Some(oldest) = upload_dirs.first() else {
        return Ok(());
    };
    match std::fs::remove_dir_all(oldest) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("remove oldest upload staging dir failed: {error}")),
    }
    upload_dirs.remove(0);
    Ok(())
}

/// A download Chromium started, tracked by its CDP guid.
#[derive(Debug, Clone, Serialize, JsonSchema)]
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
#[derive(Clone)]
pub struct LatestFrame {
    pub frame: Arc<ScreencastFrameEvent>,
    pub seq: u64,
    pub timestamp: i64,
}

type ScreencastFrameEvent = cdp_page::EventScreencastFrame;

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

/// How a session came to hold its page, which decides what shutdown may
/// destroy. A launched session is a tab in the worker's own shared Chromium:
/// shutdown closes that one target (and its incognito context). An attached
/// session holds a CDP connection into the user's own running browser:
/// shutdown closes at most the one tab the session created (`owns_page`),
/// and an adopted user tab is always released untouched. The user's browser
/// process is never closed or killed in attached mode.
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

/// The back/forward stack a tab keeps for itself. Chromium's own navigation
/// history dies with the page, so a tab that slept (or the worker restarted)
/// would otherwise lose back/forward; `browser::history` falls back to this
/// when Chromium has no entry to move to. Kept in lockstep with committed
/// navigations: a back/forward move is announced with `set_pending` so its
/// commit moves the index instead of pushing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NavStack {
    pub urls: Vec<String>,
    pub index: usize,
    #[serde(skip)]
    pending: Option<usize>,
}

impl NavStack {
    /// A top-document navigation committed to `url`.
    pub fn commit(&mut self, url: &str) {
        if url.is_empty() || url == "about:blank" {
            self.pending = None;
            return;
        }
        if let Some(target) = self.pending.take() {
            if self.urls.get(target).is_some_and(|u| u == url) {
                self.index = target;
                return;
            }
        }
        // A reload, or the page coming back after sleep: same entry.
        if self.urls.get(self.index).is_some_and(|u| u == url) {
            return;
        }
        self.urls.truncate(self.index + 1);
        self.urls.push(url.to_string());
        if self.urls.len() > NAV_CAP {
            self.urls.remove(0);
        }
        self.index = self.urls.len() - 1;
    }

    /// The entry one step back (or forward) from the current one.
    pub fn neighbour(&self, back: bool) -> Option<(usize, String)> {
        let target = if back {
            self.index.checked_sub(1)?
        } else {
            self.index + 1
        };
        self.urls.get(target).map(|u| (target, u.clone()))
    }

    /// Announce a back/forward move to `target`; the matching commit lands
    /// there instead of pushing a new entry.
    pub fn set_pending(&mut self, target: usize) {
        self.pending = Some(target);
    }
}

/// The durable tab: what the tab strip shows and what a page is reopened
/// from. Regular tabs are persisted to `tabs.json`; incognito and attached
/// tabs live only in memory.
pub struct Tab {
    pub id: String,
    /// Private tab: its page runs in a throwaway browser context, nothing
    /// is written under `data_dir`, and inactivity closes it for good.
    pub incognito: bool,
    pub read_only: bool,
    /// Bound into an external browser (`sessions::attach`); never persisted
    /// and never put to sleep — inactivity closes it.
    pub attached: bool,
    pub created_ms: i64,
    /// Optional lifetime; the tab closes once this much time passed since
    /// `created_ms`. None = lives until closed.
    pub ttl_ms: Option<u64>,
    last_used_ms: AtomicU64,
    /// Screencast frame cursor. On the tab, not the page, so a viewer that
    /// ignores frames older than the last one it saw keeps working after
    /// the tab slept and woke into a fresh page.
    frame_seq: AtomicU64,
    url: Mutex<String>,
    title: Mutex<String>,
    /// Visited pages, newest last, for the history panel. Capped.
    pub history: Mutex<Vec<HistoryVisit>>,
    /// Back/forward stack that outlives the page; see `NavStack`.
    pub nav: Mutex<NavStack>,
    /// Downloads Chromium started in this tab, by CDP guid. Empty for
    /// attached tabs, where the worker does not own the download policy.
    pub downloads: Mutex<Vec<DownloadRecord>>,
    /// Where `allowAndName` writes this tab's files (named by guid): the
    /// shared `data_dir/downloads` for regular tabs, a private temp dir for
    /// incognito ones, None for attached tabs.
    pub downloads_dir: Option<PathBuf>,
}

/// What `tabs.json` holds per regular tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TabRecord {
    id: String,
    url: String,
    title: String,
    #[serde(default)]
    read_only: bool,
    created_ms: i64,
    #[serde(default)]
    ttl_ms: Option<u64>,
    #[serde(default)]
    history: Vec<HistoryVisit>,
    #[serde(default)]
    nav: NavStack,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TabStore {
    next_slot: u64,
    tabs: Vec<TabRecord>,
}

impl Tab {
    pub fn touch(&self) {
        self.last_used_ms.store(now_ms() as u64, Ordering::Relaxed);
    }

    pub fn last_used_ms(&self) -> i64 {
        self.last_used_ms.load(Ordering::Relaxed) as i64
    }

    pub fn url(&self) -> String {
        self.url.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn title(&self) -> String {
        self.title.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_location(&self, url: &str, title: Option<&str>) {
        if !url.is_empty() {
            *self.url.lock().unwrap_or_else(|p| p.into_inner()) = url.to_string();
        }
        if let Some(title) = title {
            *self.title.lock().unwrap_or_else(|p| p.into_inner()) = title.to_string();
        }
    }

    /// A top-document navigation committed to `url`: the tab remembers
    /// where it is, the history panel and the back/forward stack learn about
    /// it. Idempotent, so the navigation pump and the handler that started
    /// the navigation can both report it — whichever runs first wins, and a
    /// pump that dies before it gets there (the tab put to sleep right after)
    /// loses nothing.
    pub fn commit_location(&self, url: &str, title: Option<&str>) {
        self.set_location(url, title);
        self.record_visit(url, title.unwrap_or_default());
        self.nav
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .commit(url);
    }

    /// Whether this tab is written to `tabs.json`.
    pub fn persists(&self) -> bool {
        !self.incognito && !self.attached
    }

    pub fn expired(&self, now: i64) -> bool {
        self.ttl_ms
            .is_some_and(|ttl| now.saturating_sub(self.created_ms) >= ttl as i64)
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

    /// Update a download's progress and state by guid. False when the guid
    /// is not this tab's (another tab in the same browser context).
    pub fn download_progress(&self, guid: &str, received: u64, total: u64, state: &str) -> bool {
        let mut downloads = self.downloads.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(record) = downloads.iter_mut().find(|d| d.guid == guid) {
            record.received_bytes = received;
            record.total_bytes = total.max(received);
            record.state = state.to_string();
            true
        } else {
            false
        }
    }

    pub fn remove_download_file(&self, guid: &str) {
        if guid.contains(['/', '\\']) || guid.contains("..") {
            return;
        }
        if let Some(dir) = &self.downloads_dir {
            let _ = std::fs::remove_file(dir.join(guid));
        }
    }

    /// Drop every file this tab downloaded (the dir itself is shared by the
    /// regular tabs and stays; an incognito tab's private dir goes with it).
    fn remove_download_files(&self) {
        let guids: Vec<String> = self
            .downloads
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|d| d.guid.clone())
            .collect();
        for guid in guids {
            self.remove_download_file(&guid);
        }
        if self.incognito {
            if let Some(dir) = &self.downloads_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    fn record(&self) -> TabRecord {
        TabRecord {
            id: self.id.clone(),
            url: self.url(),
            title: self.title(),
            read_only: self.read_only,
            created_ms: self.created_ms,
            ttl_ms: self.ttl_ms,
            history: self
                .history
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            nav: self.nav.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        }
    }
}

/// The worker's own Chromium process, shared by every launched tab. Launched
/// lazily by the first tab that needs a page and closed again when the last
/// live tab sleeps, which also flushes the profile to disk.
pub struct SharedBrowser {
    browser: tokio::sync::Mutex<Browser>,
    pub headless: bool,
    /// The browser's user agent with the `Headless` marker dropped, applied
    /// to every page: sites that refuse "HeadlessChrome" (x.com answers 400)
    /// then serve the page they serve any Chrome.
    user_agent: Mutex<String>,
    handler: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SharedBrowser {
    fn user_agent(&self) -> String {
        self.user_agent
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    async fn close(&self) {
        let mut browser = self.browser.lock().await;
        if browser.close().await.is_err() {
            if let Some(Err(e)) = browser.kill().await {
                tracing::warn!(error = %e, "browser kill failed");
            }
        }
        let _ = browser.wait().await;
        if let Some(handler) = self
            .handler
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            handler.abort();
        }
    }
}

/// Which CDP connection a live session speaks over. An attached session's
/// own connection is held here for its lifetime: dropping it would close
/// the connection under the page.
enum Backing {
    Shared(Arc<SharedBrowser>),
    Own(Box<tokio::sync::Mutex<Browser>>),
}

/// A tab with its page open. Everything here is bound to the page and dies
/// with it (a sleeping tab keeps only its `Tab`).
pub struct Session {
    pub id: String,
    pub tab: Arc<Tab>,
    pub headless: bool,
    /// Ownership model deciding what shutdown may destroy; see `SessionKind`.
    pub kind: SessionKind,
    /// Inspection-only session: interaction functions are rejected while
    /// navigation and reads stay available. Immutable for the tab's lifetime.
    pub read_only: bool,
    pub incognito: bool,
    /// The throwaway browser context an incognito page runs in.
    context_id: Option<cdp_browser::BrowserContextId>,
    backing: Backing,
    /// The live viewport the page renders at. Set at launch, then tracked to
    /// the console pane's size by browser::resize (or overridden by the
    /// device toolbar), so the streamed frame fills the pane with no
    /// letterboxing and click coordinates map 1:1.
    pub viewport_width: AtomicU32,
    pub viewport_height: AtomicU32,
    pub latest_frame: Mutex<Option<LatestFrame>>,
    /// Number of live consumers of the Chromium screencast: each console
    /// viewer and each recording counts as one. The CDP screencast runs
    /// while this is > 0; it starts on the 0->1 transition and stops on
    /// 1->0, so stopping a recording never cuts off a UI viewer (and vice
    /// versa). A watched tab is never put to sleep.
    pub screencast_consumers: std::sync::atomic::AtomicUsize,
    /// Set while a `browser::recording` is capturing; the screencast pump
    /// writes decoded frames into it.
    pub recording: tokio::sync::Mutex<Option<Recording>>,
    pub page: Page,
    pub console: Mutex<RingBuffer<ConsoleEntry>>,
    pub network: Mutex<RingBuffer<NetworkEntry>>,
    seq: AtomicU64,
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
    /// Cross-call state for `browser::execute`; lives until the page closes.
    pub exec_state: Mutex<serde_json::Value>,
    /// Serializes explicit navigation, execute, and file-input attachment.
    pub navigation_lock: tokio::sync::Mutex<()>,
    /// Loud policy denial captured by the session navigation gate for the
    /// explicit navigation or execute call currently holding the lock.
    navigation_error: Mutex<Option<String>>,
    /// File-input staging directories retained while Chromium may still read
    /// their paths, then removed on stop.
    upload_dirs: Mutex<Vec<PathBuf>>,
    upload_counter: AtomicU64,
    pick_counter: AtomicU64,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Session {
    pub fn touch(&self) {
        self.tab.touch();
    }

    pub fn last_used_ms(&self) -> i64 {
        self.tab.last_used_ms()
    }

    pub fn created_ms(&self) -> i64 {
        self.tab.created_ms
    }

    pub fn clear_navigation_error(&self) {
        *self
            .navigation_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    pub fn record_navigation_error(&self, error: String) {
        let mut slot = self
            .navigation_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    pub fn take_navigation_error(&self) -> Option<String> {
        self.navigation_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
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

    /// `Page.navigate` the way a browser does it: Chromium commits its own
    /// error page for a network failure or an empty HTTP error response, and
    /// the tab shows that page. The error text is returned for the caller to
    /// report; only a broken CDP connection is an `Err`.
    pub async fn navigate(&self, url: &str) -> Result<Option<String>, String> {
        let result = self
            .page
            .execute(cdp_page::NavigateParams::new(url))
            .await
            .map_err(|e| format!("navigation failed: {e}"))?;
        Ok(result.result.error_text.clone())
    }

    /// `navigate`, plus the fallback a browser's address bar gives a local
    /// dev server: an `https://` url on a loopback or private host that fails
    /// the TLS handshake (the server speaks plain HTTP) is retried over
    /// `http://` when that scheme is allowed. Returns the url that ended up
    /// loading and Chromium's error text, if any.
    pub async fn navigate_like_a_browser(
        &self,
        url: &str,
        allow_http: bool,
    ) -> Result<(String, Option<String>), String> {
        let error = self.navigate(url).await?;
        if let (true, Some(err)) = (allow_http, error.as_deref()) {
            if let Some(plain) = http_fallback_url(url, err) {
                let error = self.navigate(&plain).await?;
                return Ok((plain, error));
            }
        }
        Ok((url.to_string(), error))
    }

    pub fn create_upload_dir(&self) -> Result<PathBuf, String> {
        let mut upload_dirs = self
            .upload_dirs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if upload_dirs.len() >= UPLOAD_DIRS_CAP {
            remove_oldest_upload_dir(&mut upload_dirs)?;
        }
        let sequence = self.upload_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let owner = format!("{}-{sequence}", self.id);
        let path = create_private_temp_dir("iii-browser-upload", &owner, temp_nonce())
            .map_err(|error| format!("create upload staging dir failed: {error}"))?;
        upload_dirs.push(path.clone());
        Ok(path)
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

    /// Release what the worker holds for this page without talking to
    /// Chromium: finalize a recording, stop the event pumps, drop upload
    /// staging. Enough on its own when the browser is already gone.
    async fn detach(&self) {
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
        for dir in self
            .upload_dirs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Close the page. Launched: close the tab's target (and its incognito
    /// context) in the shared browser, which stays up for the other tabs.
    /// Attached: close only a tab the session itself created; an adopted
    /// user tab is released untouched, and the user's browser process is
    /// never closed. Idempotent at the caller level: `Sessions` removes the
    /// session from the live map first, so a second stop never reaches here.
    pub async fn shutdown(&self) {
        self.detach().await;
        match (&self.backing, self.kind) {
            (Backing::Shared(shared), SessionKind::Launched) => {
                let browser = shared.browser.lock().await;
                let target = self.page.target_id().clone();
                if let Err(e) = browser
                    .execute(cdp_target::CloseTargetParams::new(target))
                    .await
                {
                    tracing::debug!(session_id = %self.id, error = %e, "tab close failed");
                }
                if let Some(context) = &self.context_id {
                    let _ = browser.dispose_browser_context(context.clone()).await;
                }
            }
            (Backing::Own(browser), SessionKind::Attached { owns_page: true }) => {
                let target = self.page.target_id().clone();
                if let Err(e) = browser
                    .lock()
                    .await
                    .execute(cdp_target::CloseTargetParams::new(target))
                    .await
                {
                    tracing::debug!(session_id = %self.id, error = %e, "tab close failed");
                }
            }
            _ => {}
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

/// What `Sessions::open` takes to make a tab.
#[derive(Debug, Default, Clone)]
pub struct OpenRequest {
    pub url: Option<String>,
    /// Applies when this open launches the browser process; a running
    /// browser keeps its mode.
    pub headful: Option<bool>,
    pub read_only: bool,
    pub incognito: bool,
    pub ttl_ms: Option<u64>,
}

/// The browser: the tab list, the live pages, the shared Chromium process,
/// and everything a pump task needs.
pub struct Sessions {
    /// Every tab in strip order, live or asleep.
    tabs: Mutex<Vec<Arc<Tab>>>,
    /// Tabs with a page open, by id.
    live: Mutex<HashMap<String, Arc<Session>>>,
    counter: AtomicU64,
    browser: tokio::sync::Mutex<Option<Arc<SharedBrowser>>>,
    /// Serializes wake-ups so two calls on one sleeping tab open one page.
    activate_lock: tokio::sync::Mutex<()>,
    /// Target ids of user tabs currently adopted by a session. Adoption is
    /// exclusive: a tab in this set cannot be adopted again until its
    /// session stops.
    adopted_targets: Mutex<std::collections::HashSet<String>>,
    /// Handoffs currently paused, keyed by handoff id. A confirm removes and
    /// fires the sender; the parked `browser::handoff` handler then returns.
    pending_handoffs: Mutex<HashMap<String, PendingHandoff>>,
    handoff_counter: AtomicU64,
    /// Resolved `config.data_dir`, fixed at startup: the profile, the
    /// downloads, and `tabs.json` live under it.
    pub data_dir: PathBuf,
    pub config: SharedConfig,
    pub emitter: Arc<Emitter>,
    pub iii: Arc<iii_sdk::IIIClient>,
}

impl Sessions {
    pub fn new(
        config: SharedConfig,
        emitter: Arc<Emitter>,
        iii: Arc<iii_sdk::IIIClient>,
    ) -> Arc<Self> {
        let data_dir = iii_worker_paths::resolve_path(&config.load().data_dir);
        let sessions = Arc::new(Self {
            tabs: Mutex::new(Vec::new()),
            live: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            browser: tokio::sync::Mutex::new(None),
            activate_lock: tokio::sync::Mutex::new(()),
            adopted_targets: Mutex::new(std::collections::HashSet::new()),
            pending_handoffs: Mutex::new(HashMap::new()),
            handoff_counter: AtomicU64::new(0),
            data_dir,
            config,
            emitter,
            iii,
        });
        sessions.restore();
        sessions
    }

    pub fn profile_dir(&self) -> PathBuf {
        self.data_dir.join("profile")
    }

    fn downloads_dir(&self) -> PathBuf {
        self.data_dir.join("downloads")
    }

    fn store_path(&self) -> PathBuf {
        self.data_dir.join("tabs.json")
    }

    /// Load the regular tabs saved by the previous run, asleep; the first
    /// call on one opens its page again.
    fn restore(&self) {
        let path = self.store_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return;
        };
        let store: TabStore = match serde_json::from_str(&raw) {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "tabs.json unreadable; starting empty");
                return;
            }
        };
        let downloads_dir = self.downloads_dir();
        let mut max_slot = store.next_slot;
        let mut tabs = self.tabs.lock().unwrap_or_else(|p| p.into_inner());
        for record in store.tabs {
            if let Some(slot) = record
                .id
                .strip_prefix('b')
                .and_then(|n| n.parse::<u64>().ok())
            {
                max_slot = max_slot.max(slot);
            }
            tabs.push(Arc::new(Tab {
                id: record.id,
                incognito: false,
                read_only: record.read_only,
                attached: false,
                created_ms: record.created_ms,
                ttl_ms: record.ttl_ms,
                last_used_ms: AtomicU64::new(now_ms() as u64),
                frame_seq: AtomicU64::new(0),
                url: Mutex::new(record.url),
                title: Mutex::new(record.title),
                history: Mutex::new(record.history),
                nav: Mutex::new(record.nav),
                downloads: Mutex::new(Vec::new()),
                downloads_dir: Some(downloads_dir.clone()),
            }));
        }
        self.counter.store(max_slot, Ordering::Relaxed);
        tracing::info!(tabs = tabs.len(), "restored tabs");
    }

    /// Write the regular tabs to `tabs.json` (atomically: temp file + rename).
    pub fn persist(&self) {
        let store = TabStore {
            next_slot: self.counter.load(Ordering::Relaxed),
            tabs: self
                .list_tabs()
                .iter()
                .filter(|tab| tab.persists())
                .map(|tab| tab.record())
                .collect(),
        };
        let path = self.store_path();
        let tmp = path.with_extension("json.tmp");
        let result = std::fs::create_dir_all(&self.data_dir)
            .and_then(|_| {
                std::fs::write(&tmp, serde_json::to_vec_pretty(&store).unwrap_or_default())
            })
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            tracing::warn!(path = %path.display(), error = %e, "saving tabs failed");
        }
    }

    pub fn tab(&self, id: &str) -> Option<Arc<Tab>> {
        self.tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    /// Every tab in strip order.
    pub fn list_tabs(&self) -> Vec<Arc<Tab>> {
        self.tabs.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The live session for a tab, None while it sleeps or is unknown.
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    /// Live sessions, sorted by id.
    pub fn live_sessions(&self) -> Vec<Arc<Session>> {
        let mut sessions: Vec<_> = self
            .live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        sessions
    }

    pub fn live_count(&self) -> usize {
        self.live.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub async fn browser_running(&self) -> bool {
        self.browser.lock().await.is_some()
    }

    fn next_id(&self) -> String {
        format!("b{}", self.counter.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// Open a new tab: create its record, open its page, and (when asked)
    /// navigate it. An origin-policy denial closes the tab again and fails
    /// the open; a network-level failure leaves Chromium's error page in the
    /// tab and is reported through `open_error`.
    pub async fn open(
        self: &Arc<Self>,
        request: OpenRequest,
    ) -> Result<(Arc<Session>, Option<String>), String> {
        let downloads_dir = if request.incognito {
            None
        } else {
            let dir = self.downloads_dir();
            let _ = std::fs::create_dir_all(&dir);
            Some(dir)
        };
        let id = self.next_id();
        let downloads_dir = match downloads_dir {
            Some(dir) => Some(dir),
            None => create_private_temp_dir("iii-browser-dl", &id, temp_nonce()).ok(),
        };
        let tab = Arc::new(Tab {
            id: id.clone(),
            incognito: request.incognito,
            read_only: request.read_only,
            attached: false,
            created_ms: now_ms(),
            ttl_ms: request.ttl_ms,
            last_used_ms: AtomicU64::new(now_ms() as u64),
            frame_seq: AtomicU64::new(0),
            url: Mutex::new("about:blank".to_string()),
            title: Mutex::new(String::new()),
            history: Mutex::new(Vec::new()),
            nav: Mutex::new(NavStack::default()),
            downloads: Mutex::new(Vec::new()),
            downloads_dir,
        });
        self.tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(tab.clone());
        self.persist();

        let session = match self.wake(&tab, request.headful).await {
            Ok(session) => session,
            Err(error) => {
                self.stop(&id, "stopped").await;
                return Err(error);
            }
        };
        let mut open_error = None;
        if let Some(url) = request.url.as_deref() {
            session.clear_navigation_error();
            let allow_http = self
                .config
                .load()
                .allowed_schemes
                .iter()
                .any(|s| s == "http");
            let navigation = session.navigate_like_a_browser(url, allow_http).await;
            if let Some(policy_error) = session.take_navigation_error() {
                self.stop(&id, "stopped").await;
                return Err(policy_error);
            }
            match navigation {
                Ok((loaded, error)) => {
                    open_error = error;
                    tab.set_location(&loaded, None);
                }
                Err(error) => {
                    self.stop(&id, "stopped").await;
                    return Err(error);
                }
            }
        }
        Ok((session, open_error))
    }

    /// The live session for a tab, opening its page again if it sleeps.
    /// Any function call on a tab counts as selecting it.
    pub async fn activate(self: &Arc<Self>, id: &str) -> Result<Arc<Session>, String> {
        if let Some(session) = self.get(id) {
            session.touch();
            return Ok(session);
        }
        let tab = self.tab(id).ok_or_else(|| {
            format!("unknown session '{id}'; list tabs with browser::sessions::list")
        })?;
        if tab.attached {
            return Err(format!(
                "session '{id}' was attached to an external browser and has closed"
            ));
        }
        let session = self.wake(&tab, None).await?;
        let url = tab.url();
        if url != "about:blank" && !url.is_empty() {
            session.clear_navigation_error();
            let navigation = session.navigate(&url).await;
            let _ = session.take_navigation_error();
            // The caller is about to act on the page: give the load the
            // default navigation budget before handing the tab over.
            if matches!(navigation, Ok(None)) {
                let wait = std::time::Duration::from_millis(self.config.load().default_timeout_ms);
                let _ = tokio::time::timeout(wait, session.page.wait_for_navigation()).await;
            }
        }
        Ok(session)
    }

    /// Open the page for `tab` in the shared browser (launching it if needed)
    /// and register the session as live. Serialized so a burst of calls on
    /// one sleeping tab opens one page.
    async fn wake(
        self: &Arc<Self>,
        tab: &Arc<Tab>,
        headful: Option<bool>,
    ) -> Result<Arc<Session>, String> {
        let _guard = self.activate_lock.lock().await;
        if let Some(session) = self.get(&tab.id) {
            return Ok(session);
        }
        self.make_room(&tab.id).await?;
        let shared = self.ensure_browser(headful).await?;
        let cfg = self.config.load_full();
        let origin_gate_enabled = origin_gate_configured(&cfg);

        let context_id = if tab.incognito {
            let params = cdp_target::CreateBrowserContextParams::builder()
                .dispose_on_detach(true)
                .build();
            Some(
                shared
                    .browser
                    .lock()
                    .await
                    .create_browser_context(params)
                    .await
                    .map_err(|e| format!("failed to open an incognito context: {e}"))?,
            )
        } else {
            None
        };
        // Every tab gets its own headless window: a window shows only its
        // active tab, so a second tab in the same window would hide the
        // first — no repaints, no screencast frames, a frozen picture.
        let mut params = cdp_target::CreateTargetParams::new("about:blank");
        params.browser_context_id = context_id.clone();
        params.new_window = Some(true);
        let page = match shared.browser.lock().await.new_page(params).await {
            Ok(page) => page,
            Err(e) => {
                if let Some(context) = &context_id {
                    let _ = shared
                        .browser
                        .lock()
                        .await
                        .dispose_browser_context(context.clone())
                        .await;
                }
                return Err(format!("failed to open page: {e}"));
            }
        };

        let _ = page.set_user_agent(shared.user_agent()).await;
        // Log + Network are not enabled by default; console API events are.
        let _ = page.execute(cdp_log::EnableParams::default()).await;
        let _ = page.execute(cdp_network::EnableParams::default()).await;
        // Downloads land in the tab's download dir, named by guid, with
        // progress events. Best-effort: a browser that refuses leaves
        // downloads disabled, not the tab broken. The behavior is per
        // browser context, so an incognito tab names its own.
        if let Some(downloads_dir) = &tab.downloads_dir {
            let mut behavior = cdp_browser::SetDownloadBehaviorParams::builder()
                .behavior(cdp_browser::SetDownloadBehaviorBehavior::AllowAndName)
                .download_path(downloads_dir.to_string_lossy().to_string())
                .events_enabled(true);
            if let Some(context) = &context_id {
                behavior = behavior.browser_context_id(context.clone());
            }
            if let Ok(params) = behavior.build() {
                let _ = page.execute(params).await;
            }
        }

        let session = Arc::new(Session {
            id: tab.id.clone(),
            tab: tab.clone(),
            headless: shared.headless,
            kind: SessionKind::Launched,
            read_only: tab.read_only,
            incognito: tab.incognito,
            context_id,
            backing: Backing::Shared(shared.clone()),
            viewport_width: AtomicU32::new(cfg.viewport_width),
            viewport_height: AtomicU32::new(cfg.viewport_height),
            latest_frame: Mutex::new(None),
            screencast_consumers: std::sync::atomic::AtomicUsize::new(0),
            recording: tokio::sync::Mutex::new(None),
            page: page.clone(),
            console: Mutex::new(RingBuffer::new(cfg.console_buffer as usize)),
            network: Mutex::new(RingBuffer::new(cfg.network_buffer as usize)),
            seq: AtomicU64::new(1),
            refs: Mutex::new(HashMap::new()),
            ref_counter: AtomicU64::new(0),
            generation: AtomicU64::new(1),
            snapshot_keys: Mutex::new(None),
            exec_state: Mutex::new(serde_json::Value::Object(serde_json::Map::new())),
            navigation_lock: tokio::sync::Mutex::new(()),
            navigation_error: Mutex::new(None),
            upload_dirs: Mutex::new(Vec::new()),
            upload_counter: AtomicU64::new(0),
            pick_counter: AtomicU64::new(0),
            tasks: Mutex::new(Vec::new()),
        });

        let pumps =
            match spawn_event_pumps(self.clone(), session.clone(), origin_gate_enabled).await {
                Ok(pumps) => pumps,
                Err(error) => {
                    session.shutdown().await;
                    return Err(error);
                }
            };
        session
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(pumps);
        tab.touch();
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(tab.id.clone(), session.clone());
        self.emit_updated(tab, true).await;
        Ok(session)
    }

    /// Put unwatched tabs to sleep, least recently used first, until a page
    /// for `for_id` fits under `max_sessions`. Fails only when every live
    /// tab is being watched or recorded.
    async fn make_room(self: &Arc<Self>, for_id: &str) -> Result<(), String> {
        let cap = self.config.load().max_sessions as usize;
        loop {
            if self.live_count() < cap {
                return Ok(());
            }
            let mut candidates: Vec<Arc<Session>> = Vec::new();
            for session in self.live_sessions() {
                if session.id != for_id
                    && !session.screencast_on()
                    && session.recording.lock().await.is_none()
                {
                    candidates.push(session);
                }
            }
            candidates.sort_by_key(|s| s.last_used_ms());
            let Some(victim) = candidates.first() else {
                return Err(format!(
                    "tab limit reached ({cap}) and every live tab is being watched; close one \
                     with browser::sessions::stop or raise max_sessions"
                ));
            };
            tracing::info!(session_id = %victim.id, "sleeping tab to make room");
            self.sleep_inner(&victim.id.clone(), "idle", false).await;
        }
    }

    /// The shared Chromium, launched on first use. `headful` overrides the
    /// configured mode for a launch; a running browser keeps its mode.
    async fn ensure_browser(
        self: &Arc<Self>,
        headful: Option<bool>,
    ) -> Result<Arc<SharedBrowser>, String> {
        let mut slot = self.browser.lock().await;
        if let Some(shared) = slot.as_ref() {
            return Ok(shared.clone());
        }
        let cfg = self.config.load_full();
        let headless = match headful {
            Some(headful) => !headful,
            None => cfg.headless,
        };
        let profile = self.profile_dir();
        std::fs::create_dir_all(&profile)
            .map_err(|e| format!("cannot create profile dir {}: {e}", profile.display()))?;
        let browser_config = build_browser_config(&cfg, headless, &profile)?;
        let (browser, mut handler) = match Browser::launch(browser_config).await {
            Ok(launched) => launched,
            Err(first) if reap_orphan_chromium(&profile) => {
                tracing::warn!(error = %first, "launch failed against an orphaned Chromium; reaped it, retrying");
                Browser::launch(build_browser_config(&cfg, headless, &profile)?)
                    .await
                    .map_err(|e| format!("failed to launch Chromium: {e}"))?
            }
            Err(e) => return Err(format!("failed to launch Chromium: {e}")),
        };
        let shared = Arc::new(SharedBrowser {
            browser: tokio::sync::Mutex::new(browser),
            headless,
            user_agent: Mutex::new(String::new()),
            handler: Mutex::new(None),
        });
        // The handler pump ends when the CDP connection drops: a clean
        // close (the slot was already emptied) or Chromium dying underneath
        // us, in which case every live tab goes to sleep so the next call
        // relaunches instead of talking to a corpse.
        let sessions = Arc::downgrade(self);
        let mine = Arc::downgrade(&shared);
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::debug!(error = %e, "cdp handler event error");
                }
            }
            if let (Some(sessions), Some(mine)) = (sessions.upgrade(), mine.upgrade()) {
                sessions.on_browser_lost(&mine).await;
            }
        });
        *shared.handler.lock().unwrap_or_else(|p| p.into_inner()) = Some(handler_task);
        // Only now that the handler pump runs can the browser answer.
        let user_agent = shared
            .browser
            .lock()
            .await
            .user_agent()
            .await
            .map(|ua| ua.replace("HeadlessChrome", "Chrome"))
            .unwrap_or_default();
        *shared.user_agent.lock().unwrap_or_else(|p| p.into_inner()) = user_agent;
        *slot = Some(shared.clone());
        tracing::info!(headless, profile = %profile.display(), "browser launched");
        Ok(shared)
    }

    /// Chromium went away without us closing it: forget every launched
    /// session (their pages are gone) and the browser, keeping the tabs.
    async fn on_browser_lost(self: &Arc<Self>, lost: &Arc<SharedBrowser>) {
        {
            let mut slot = self.browser.lock().await;
            match slot.as_ref() {
                Some(current) if Arc::ptr_eq(current, lost) => {
                    *slot = None;
                }
                _ => return,
            }
        }
        tracing::warn!("browser process exited; live tabs are asleep until used again");
        for session in self.live_sessions() {
            if session.kind != SessionKind::Launched {
                continue;
            }
            self.live
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&session.id);
            session.detach().await;
            if session.tab.persists() {
                self.emit_updated(&session.tab, false).await;
            } else {
                self.remove_tab(&session.id);
                self.emit_stopped(&session.id, "crashed").await;
            }
        }
    }

    /// Close the shared browser once no launched tab has a page open, which
    /// also flushes the profile (cookies, storage) to disk.
    async fn shutdown_browser_if_idle(&self) {
        let still_live = self
            .live_sessions()
            .iter()
            .any(|s| s.kind == SessionKind::Launched);
        if still_live {
            return;
        }
        let taken = self.browser.lock().await.take();
        if let Some(shared) = taken {
            shared.close().await;
            tracing::info!("browser closed (no live tabs)");
        }
    }

    async fn emit_updated(&self, tab: &Tab, active: bool) {
        self.emitter
            .emit(
                EventKind::SessionUpdated,
                &tab.id,
                &SessionUpdatedEvent {
                    session_id: tab.id.clone(),
                    active,
                    url: tab.url(),
                    title: tab.title(),
                    timestamp: now_ms(),
                },
            )
            .await;
    }

    async fn emit_stopped(&self, id: &str, reason: &str) {
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
    }

    fn remove_tab(&self, id: &str) -> Option<Arc<Tab>> {
        let mut tabs = self.tabs.lock().unwrap_or_else(|p| p.into_inner());
        let index = tabs.iter().position(|t| t.id == id)?;
        Some(tabs.remove(index))
    }

    /// Put a tab to sleep: close its page, keep the tab. Incognito and
    /// attached tabs cannot sleep — they close (`stop`) instead.
    pub async fn sleep(self: &Arc<Self>, id: &str, reason: &str) {
        self.sleep_inner(id, reason, true).await;
    }

    async fn sleep_inner(self: &Arc<Self>, id: &str, reason: &str, close_browser: bool) {
        let Some(session) = self.get(id) else {
            return;
        };
        if !session.tab.persists() {
            self.stop_inner(id, reason, close_browser).await;
            return;
        }
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        session.shutdown().await;
        self.emit_updated(&session.tab, false).await;
        if close_browser {
            self.shutdown_browser_if_idle().await;
        }
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
        if self.live_count() as u64 >= cfg.max_sessions {
            return Err(format!(
                "tab limit reached ({}); stop one with browser::sessions::stop",
                cfg.max_sessions
            ));
        }
        let origin_gate_enabled = origin_gate_configured(&cfg);

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
                .new_page(if origin_gate_enabled {
                    "about:blank"
                } else {
                    url.as_deref().unwrap_or("about:blank")
                })
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

        let id = self.next_id();
        let now = now_ms();
        let tab = Arc::new(Tab {
            id: id.clone(),
            incognito: false,
            read_only,
            attached: true,
            created_ms: now,
            ttl_ms: None,
            last_used_ms: AtomicU64::new(now as u64),
            frame_seq: AtomicU64::new(0),
            url: Mutex::new(url.clone().unwrap_or_else(|| "about:blank".to_string())),
            title: Mutex::new(String::new()),
            history: Mutex::new(Vec::new()),
            nav: Mutex::new(NavStack::default()),
            downloads: Mutex::new(Vec::new()),
            downloads_dir: None,
        });
        let session = Arc::new(Session {
            id: id.clone(),
            tab: tab.clone(),
            headless: false,
            kind: SessionKind::Attached { owns_page },
            read_only,
            incognito: false,
            context_id: None,
            backing: Backing::Own(Box::new(tokio::sync::Mutex::new(browser))),
            viewport_width: AtomicU32::new(cfg.viewport_width),
            viewport_height: AtomicU32::new(cfg.viewport_height),
            latest_frame: Mutex::new(None),
            screencast_consumers: std::sync::atomic::AtomicUsize::new(0),
            recording: tokio::sync::Mutex::new(None),
            page: page.clone(),
            console: Mutex::new(RingBuffer::new(cfg.console_buffer as usize)),
            network: Mutex::new(RingBuffer::new(cfg.network_buffer as usize)),
            seq: AtomicU64::new(1),
            refs: Mutex::new(HashMap::new()),
            ref_counter: AtomicU64::new(0),
            generation: AtomicU64::new(1),
            snapshot_keys: Mutex::new(None),
            exec_state: Mutex::new(serde_json::Value::Object(serde_json::Map::new())),
            navigation_lock: tokio::sync::Mutex::new(()),
            navigation_error: Mutex::new(None),
            upload_dirs: Mutex::new(Vec::new()),
            upload_counter: AtomicU64::new(0),
            pick_counter: AtomicU64::new(0),
            tasks: Mutex::new(vec![handler_task]),
        });

        let pumps =
            match spawn_event_pumps(self.clone(), session.clone(), origin_gate_enabled).await {
                Ok(pumps) => pumps,
                Err(error) => {
                    self.release_adopted_page(&session);
                    session.shutdown().await;
                    return Err(error);
                }
            };
        session
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(pumps);

        if origin_gate_enabled && owns_page {
            if let Some(url) = url.as_deref() {
                session.clear_navigation_error();
                let navigation = session.navigate(url).await;
                let policy_error = session.take_navigation_error();
                if let Some(error) = policy_error {
                    session.shutdown().await;
                    return Err(error);
                }
                if let Err(error) = navigation {
                    session.shutdown().await;
                    return Err(format!("failed to open tab: {error}"));
                }
            }
        }

        self.tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(tab);
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, session.clone());
        Ok(session)
    }

    /// Connect to a running browser and report its open tabs (url, title,
    /// and whether a session already adopted each). Read-only: opens a
    /// throwaway CDP connection, lists, and drops it without touching any
    /// tab. Used by `browser::tabs::list`.
    pub async fn remote_tabs(&self, cdp_url: &str) -> Result<Vec<TabInfo>, String> {
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

    fn release_adopted_page(&self, session: &Session) {
        if let SessionKind::Attached { owns_page: false } = session.kind {
            let target = session.page.target_id().as_ref().to_string();
            self.adopted_targets
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&target);
        }
    }

    /// Close a tab for good: its page (if open), its record, its downloads.
    /// Returns whether the tab existed — closing an unknown id succeeds
    /// (delete semantics: the caller wants it gone, and it is).
    pub async fn stop(self: &Arc<Self>, id: &str, reason: &str) -> bool {
        self.stop_inner(id, reason, true).await
    }

    async fn stop_inner(self: &Arc<Self>, id: &str, reason: &str, close_browser: bool) -> bool {
        let session = self
            .live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        let tab = self.remove_tab(id);
        if let Some(session) = &session {
            self.release_adopted_page(session);
            // Drop any handoff parked on this session so its call unblocks
            // (its receiver errors) instead of waiting for the full timeout
            // against a dead page.
            self.pending_handoffs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .retain(|_, h| h.session_id != id);
            session.shutdown().await;
        }
        let Some(tab) = tab else {
            return false;
        };
        tab.remove_download_files();
        if tab.persists() {
            self.persist();
        }
        self.emit_stopped(id, reason).await;
        if close_browser {
            self.shutdown_browser_if_idle().await;
        }
        true
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
        session: &Arc<Session>,
        path: &str,
        codec: &str,
    ) -> Result<(), String> {
        // Hold the recording lock across the whole check-and-set so two
        // concurrent starts cannot both spawn ffmpeg and overwrite each
        // other's Recording (leaking a process).
        let mut guard = session.recording.lock().await;
        if guard.is_some() {
            return Err(format!(
                "session '{}' is already recording; stop it first",
                session.id
            ));
        }

        // Acquire a screencast consumer (starts CDP screencast if we are the
        // first). Released again on any failure below.
        self.acquire_screencast(session).await?;

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
                self.release_screencast(session).await;
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
                self.release_screencast(session).await;
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

    /// Release a screencast consumer. Stops the Chromium screencast on the
    /// 1->0 transition, leaving it running while any other consumer remains
    /// (so stopping a recording never cuts off a UI viewer). Never
    /// underflows. Watching counts as using the tab, so the release also
    /// touches it — the inactivity clock starts when the viewer leaves.
    pub async fn release_screencast(&self, session: &Arc<Session>) {
        session.touch();
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

    /// Close expired tabs and put unused, unwatched tabs to sleep (incognito
    /// and attached ones close). Called from the sweep task in `main`.
    pub async fn sweep_idle(self: &Arc<Self>) {
        let now = now_ms();
        for tab in self.list_tabs() {
            if tab.expired(now) {
                tracing::info!(session_id = %tab.id, "closing expired tab");
                self.stop(&tab.id, "expired").await;
            }
        }
        let inactive_after_ms = self.config.load().inactive_after_ms;
        if inactive_after_ms == 0 {
            return;
        }
        let cutoff = now - inactive_after_ms as i64;
        for session in self.live_sessions() {
            if session.screencast_on()
                || session.recording.lock().await.is_some()
                || session.last_used_ms() >= cutoff
            {
                continue;
            }
            tracing::info!(session_id = %session.id, "sleeping idle tab");
            self.sleep(&session.id, "idle").await;
        }
    }

    /// The browser's "Clear browsing data" for everything: close every live
    /// page, quit Chromium, and delete the profile and downloads on disk.
    /// Tabs stay (asleep) and reopen into a clean profile. Returns how many
    /// tabs were live.
    pub async fn clear_browser_data(self: &Arc<Self>) -> Result<usize, String> {
        let _guard = self.activate_lock.lock().await;
        let live = self.live_sessions();
        let count = live.len();
        for session in live {
            self.sleep_inner(&session.id, "stopped", false).await;
        }
        let taken = self.browser.lock().await.take();
        if let Some(shared) = taken {
            shared.close().await;
        }
        for dir in [self.profile_dir(), self.downloads_dir()] {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("removing {} failed: {e}", dir.display())),
            }
        }
        for tab in self.list_tabs() {
            tab.downloads
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
        }
        Ok(count)
    }

    /// Worker shutdown: close every page and the browser. Regular tabs stay
    /// in `tabs.json` and come back asleep on the next boot.
    pub async fn stop_all(self: &Arc<Self>) {
        for session in self.live_sessions() {
            self.sleep_inner(&session.id, "stopped", false).await;
        }
        let taken = self.browser.lock().await.take();
        if let Some(shared) = taken {
            shared.close().await;
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

/// One profile for the whole browser, like a browser: Chromium holds a
/// process singleton per profile, which is exactly why every tab shares one
/// process instead of getting its own.
fn build_browser_config(
    cfg: &WorkerConfig,
    headless: bool,
    profile_dir: &std::path::Path,
) -> Result<BrowserConfig, String> {
    let mut builder = BrowserConfig::builder()
        .window_size(cfg.viewport_width, cfg.viewport_height)
        .viewport(chromiumoxide::handler::viewport::Viewport {
            width: cfg.viewport_width,
            height: cfg.viewport_height,
            ..Default::default()
        })
        .user_data_dir(profile_dir);
    builder = if headless {
        builder.new_headless_mode()
    } else {
        builder.with_head()
    };
    if !cfg.executable.is_empty() {
        builder = builder.chrome_executable(&cfg.executable);
    }
    builder.build()
}

/// A worker killed without cleanup leaves its Chromium alive holding the
/// profile's singleton lock; the next launch hands off to that orphan and
/// times out. When the lock names a process that is running on THIS profile,
/// kill it so the caller can launch again. Nothing else is ever touched.
#[cfg(not(unix))]
fn reap_orphan_chromium(_profile: &std::path::Path) -> bool {
    false
}

#[cfg(unix)]
fn reap_orphan_chromium(profile: &std::path::Path) -> bool {
    let Ok(lock) = std::fs::read_link(profile.join("SingletonLock")) else {
        return false;
    };
    // The link target is `<hostname>-<pid>`.
    let Some(pid) = lock
        .to_string_lossy()
        .rsplit('-')
        .next()
        .and_then(|p| p.parse::<i32>().ok())
    else {
        return false;
    };
    let Ok(ps) = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    let command = String::from_utf8_lossy(&ps.stdout);
    if !command.contains(&*profile.to_string_lossy()) {
        return false;
    }
    // SAFETY: plain libc call on a pid we just verified runs Chromium on our
    // own profile directory.
    let killed = unsafe { libc::kill(pid, libc::SIGKILL) } == 0;
    if killed {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = std::fs::remove_file(profile.join("SingletonLock"));
    }
    killed
}

/// The `http://` twin of an `https://` url on a local host whose TLS
/// handshake failed — `https://localhost:3000` against a plain dev server
/// answers `net::ERR_SSL_PROTOCOL_ERROR`. Public hosts never downgrade.
fn http_fallback_url(url: &str, error: &str) -> Option<String> {
    let tls_failure = error.starts_with("net::ERR_SSL_")
        || error.starts_with("net::ERR_CERT_")
        || matches!(
            error,
            "net::ERR_CONNECTION_CLOSED" | "net::ERR_CONNECTION_RESET" | "net::ERR_EMPTY_RESPONSE"
        );
    if !tls_failure {
        return None;
    }
    let mut parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let local = match parsed.host()? {
        url::Host::Domain(host) => host == "localhost" || host.ends_with(".localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    };
    if !local {
        return None;
    }
    let port = parsed.port();
    parsed.set_scheme("http").ok()?;
    // `set_scheme` keeps an explicit port; a bare https url moves to 80.
    parsed.set_port(port).ok()?;
    Some(parsed.to_string())
}

fn origin_gate_configured(config: &WorkerConfig) -> bool {
    config.origin_policies.is_some() || config.default_origin_policy.is_some()
}

fn origin_access_denial(config: &WorkerConfig, url: &str) -> Option<String> {
    if origin_policy_for(config, url).access {
        return None;
    }
    Some(format!(
        "origin '{}' is denied by {} (access)",
        origin_label(url),
        origin_policy_config_key_for(config, url)
    ))
}

/// A top-document navigation committed: the tab remembers where it is, the
/// history panel and the back/forward stack learn about it, subscribers
/// hear `browser::navigated`, and regular tabs are saved.
async fn note_navigation(sessions: &Arc<Sessions>, session: &Arc<Session>, url: &str) {
    // A wedged renderer must not stall the navigation pump on a title read;
    // after a short wait the visit records untitled.
    let title = tokio::time::timeout(std::time::Duration::from_secs(2), session.page.get_title())
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten()
        .unwrap_or_default();
    session.tab.commit_location(url, Some(&title));
    if session.tab.persists() {
        sessions.persist();
    }
    sessions
        .emitter
        .emit(
            EventKind::Navigated,
            &session.id,
            &NavigatedEvent {
                session_id: session.id.clone(),
                url: url.to_string(),
                timestamp: now_ms(),
            },
        )
        .await;
}

/// Arm the per-session CDP event listeners. Each pump owns one event stream,
/// pushes into the ring buffer, and (console + pick) forwards to trigger
/// subscribers.
async fn spawn_event_pumps(
    sessions: Arc<Sessions>,
    session: Arc<Session>,
    origin_gate_enabled: bool,
) -> Result<Vec<tokio::task::JoinHandle<()>>, String> {
    let mut tasks = Vec::new();
    let page = &session.page;
    let main_frame = page
        .mainframe()
        .await
        .map_err(|error| format!("failed to read the main frame: {error}"))?;

    if origin_gate_enabled {
        let main_frame = main_frame
            .clone()
            .ok_or_else(|| "origin policy gate found no main frame".to_string())?;
        let mut events = page
            .event_listener::<cdp_fetch::EventRequestPaused>()
            .await
            .map_err(|error| format!("origin policy gate failed to listen: {error}"))?;
        let document_pattern = cdp_fetch::RequestPattern::builder()
            .resource_type(cdp_network::ResourceType::Document)
            .build();
        page.execute(
            cdp_fetch::EnableParams::builder()
                .pattern(document_pattern)
                .build(),
        )
        .await
        .map_err(|error| format!("origin policy gate failed to enable: {error}"))?;

        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let is_top_document = event.resource_type == cdp_network::ResourceType::Document
                    && event.frame_id == main_frame;
                if !is_top_document {
                    let _ = s
                        .page
                        .execute(cdp_fetch::ContinueRequestParams::new(
                            event.request_id.clone(),
                        ))
                        .await;
                    continue;
                }

                let denial = {
                    let config = sx.config.load();
                    origin_access_denial(&config, &event.request.url)
                };
                if let Some(error) = denial {
                    s.record_navigation_error(error);
                    if let Err(command_error) = s
                        .page
                        .execute(cdp_fetch::FailRequestParams::new(
                            event.request_id.clone(),
                            cdp_network::ErrorReason::BlockedByClient,
                        ))
                        .await
                    {
                        tracing::warn!(
                            session_id = %s.id,
                            error = %command_error,
                            "origin policy request failure command failed"
                        );
                    }
                } else if let Err(command_error) = s
                    .page
                    .execute(cdp_fetch::ContinueRequestParams::new(
                        event.request_id.clone(),
                    ))
                    .await
                {
                    tracing::debug!(
                        session_id = %s.id,
                        error = %command_error,
                        "origin policy request continue failed"
                    );
                }
            }
        }));
    }

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
    if let Ok(mut events) = page.event_listener::<cdp_page::EventFrameNavigated>().await {
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
                note_navigation(&sx, &s, &event.frame.url).await;
            }
        }));
    }

    // same-document navigations (hash routes, pushState): single-page apps
    // move this way, so they count as visits and emit the same event
    if let Ok(mut events) = page
        .event_listener::<cdp_page::EventNavigatedWithinDocument>()
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
                note_navigation(&sx, &s, &event.url).await;
            }
        }));
    }

    // screencast frames: keep only the newest, ack every frame so Chromium
    // keeps pushing. Delivery to the console (`browser::frame-event`, one
    // awaited trigger per viewer) runs on its own task fed by a watch
    // channel — latest frame wins — so a slow bus never stalls the pump or
    // builds a backlog of stale pictures; the viewer always gets the newest
    // frame the engine can take.
    if let Ok(mut events) = page
        .event_listener::<cdp_page::EventScreencastFrame>()
        .await
    {
        let (frame_tx, mut frame_rx) = tokio::sync::watch::channel::<Option<LatestFrame>>(None);
        {
            let s = session.clone();
            let sx = sessions.clone();
            tasks.push(tokio::spawn(async move {
                while frame_rx.changed().await.is_ok() {
                    let next = frame_rx.borrow_and_update().clone();
                    if let Some(frame) = next {
                        let data: &str = frame.frame.data.as_ref();
                        sx.emitter
                            .emit_awaited(
                                EventKind::FrameEvent,
                                &s.id,
                                &FrameEventPayload {
                                    session_id: s.id.clone(),
                                    frame: data.to_string(),
                                    width: frame.width(),
                                    height: frame.height(),
                                    frame_seq: frame.seq,
                                    timestamp: frame.timestamp,
                                },
                            )
                            .await;
                    }
                }
            }));
        }
        let s = session.clone();
        tasks.push(tokio::spawn(async move {
            let mut last_push_ms = 0i64;
            // Trailing-edge throttle: a frame that arrives inside the
            // interval waits here and goes out when the interval elapses
            // unless a newer one replaced it, so the final frame of a burst
            // (the settled page after a resize or a navigation) is always
            // delivered. Dropping it instead left a stale picture on a
            // static page.
            let mut held: Option<(Arc<ScreencastFrameEvent>, Vec<u8>)> = None;
            loop {
                let wait = std::time::Duration::from_millis(
                    (FRAME_MIN_INTERVAL_MS - (now_ms() - last_push_ms))
                        .clamp(0, FRAME_MIN_INTERVAL_MS) as u64,
                );
                let next = tokio::select! {
                    event = events.next() => match event {
                        Some(event) => Some(event),
                        None => break,
                    },
                    _ = tokio::time::sleep(wait), if held.is_some() => None,
                };
                if let Some(event) = next {
                    let ack = cdp_page::ScreencastFrameAckParams::new(event.session_id);
                    if let Err(e) = s.page.execute(ack).await {
                        tracing::debug!(session_id = %s.id, error = %e, "screencast ack failed");
                    }
                    let data: &str = event.data.as_ref();
                    let Ok(bytes) = STANDARD.decode(data) else {
                        continue;
                    };
                    // Right after a (re)start Chromium can emit one frame
                    // rendered at the previous surface size while its
                    // metadata already reports the new viewport. Pushed, it
                    // would show letterboxed and map clicks off by the
                    // difference; the corrected frame follows at once.
                    if !frame_matches_metadata(&event, &bytes) {
                        continue;
                    }
                    held = Some((event, bytes));
                    if now_ms() - last_push_ms < FRAME_MIN_INTERVAL_MS {
                        continue;
                    }
                }
                let Some((event, bytes)) = held.take() else {
                    continue;
                };
                last_push_ms = now_ms();
                let seq = s.tab.frame_seq.fetch_add(1, Ordering::Relaxed) + 1;
                let stream_frame = LatestFrame {
                    frame: event,
                    seq,
                    timestamp: last_push_ms,
                };
                // Offer it to the pusher (the console is bound to the frame
                // trigger); the in-memory slot stays as the seed for a fresh
                // viewer's first paint.
                let _ = frame_tx.send(Some(stream_frame.clone()));
                // Feed the recorder the same capped frame flow through a
                // bounded queue with try_send: the pump never awaits ffmpeg
                // I/O, and frames are dropped rather than backing up if the
                // encoder falls behind.
                {
                    let rec = s.recording.lock().await;
                    if let Some(recording) = rec.as_ref() {
                        let _ = recording.tx.try_send(bytes);
                    }
                }
                let mut slot = s.latest_frame.lock().unwrap_or_else(|p| p.into_inner());
                *slot = Some(stream_frame);
            }
        }));
    }

    // downloads: guid-named files land in the tab's download dir; the
    // begin/progress events feed the downloads panel. Only armed when the
    // worker owns the download policy. The behavior is per browser context,
    // so every regular tab hears every regular tab's downloads: the begin
    // event's frame id keeps each tab to its own.
    if session.tab.downloads_dir.is_some() {
        if let Ok(mut events) = page
            .event_listener::<cdp_browser::EventDownloadWillBegin>()
            .await
        {
            let s = session.clone();
            let sx = sessions.clone();
            tasks.push(tokio::spawn(async move {
                while let Some(event) = events.next().await {
                    if main_frame.as_ref().is_some_and(|id| *id != event.frame_id) {
                        continue;
                    }
                    if !origin_policy_for(&sx.config.load(), &event.url).downloads {
                        let _ = s
                            .page
                            .execute(cdp_browser::CancelDownloadParams::new(event.guid.clone()))
                            .await;
                        s.tab.remove_download_file(&event.guid);
                        continue;
                    }
                    s.tab
                        .download_begin(&event.guid, &event.suggested_filename, &event.url);
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
                    // Progress events carry no frame; a guid this tab never
                    // recorded belongs to another tab (or was refused).
                    let recorded = s.tab.download_progress(
                        &event.guid,
                        event.received_bytes as u64,
                        event.total_bytes as u64,
                        state,
                    );
                    if !recorded {
                        continue;
                    }
                    if state != "in_progress" || last_emit.elapsed().as_millis() >= 250 {
                        last_emit = std::time::Instant::now();
                        emit_download_changed(&sx, &s).await;
                    }
                }
            }));
        }
    }

    Ok(tasks)
}

/// Whether the JPEG's pixel aspect matches the viewport its metadata claims
/// (the frame is always CSS-pixel sized, whatever the device scale factor).
fn frame_matches_metadata(event: &ScreencastFrameEvent, jpeg: &[u8]) -> bool {
    let Ok(Ok((width, height))) = image::ImageReader::new(std::io::Cursor::new(jpeg))
        .with_guessed_format()
        .map(|reader| reader.into_dimensions())
    else {
        return true;
    };
    let (meta_w, meta_h) = (event.metadata.device_width, event.metadata.device_height);
    if meta_w <= 0.0 || meta_h <= 0.0 || width == 0 || height == 0 {
        return true;
    }
    let jpeg_aspect = width as f64 / height as f64;
    let meta_aspect = meta_w / meta_h;
    (jpeg_aspect - meta_aspect).abs() / meta_aspect < 0.02
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

    #[test]
    fn private_temp_dir_is_fresh_and_owner_only() {
        let owner = format!("test-{}", temp_nonce());
        let nonce = temp_nonce();
        let path = create_private_temp_dir("iii-browser-dir-test", &owner, nonce).unwrap();
        assert!(create_private_temp_dir("iii-browser-dir-test", &owner, nonce).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn upload_dir_cap_removes_the_oldest_directory() {
        let owner = format!("upload-cap-test-{}", temp_nonce());
        let root = create_private_temp_dir("iii-browser-dir-test", &owner, temp_nonce()).unwrap();
        let mut upload_dirs = Vec::new();
        for index in 0..UPLOAD_DIRS_CAP {
            let path = root.join(index.to_string());
            std::fs::create_dir(&path).unwrap();
            upload_dirs.push(path);
        }
        let oldest = upload_dirs[0].clone();

        remove_oldest_upload_dir(&mut upload_dirs).unwrap();

        assert_eq!(upload_dirs.len(), UPLOAD_DIRS_CAP - 1);
        assert!(!oldest.exists());
        assert_eq!(upload_dirs[0], root.join("1"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The stack that survives sleep: pushes forward, collapses reloads,
    /// truncates the forward branch on a fresh navigation, and follows an
    /// announced back/forward move instead of pushing.
    #[test]
    fn nav_stack_tracks_back_and_forward_across_commits() {
        let mut nav = NavStack::default();
        nav.commit("about:blank");
        assert!(nav.urls.is_empty());
        nav.commit("https://a.test/");
        nav.commit("https://b.test/");
        nav.commit("https://b.test/"); // reload
        nav.commit("https://c.test/");
        assert_eq!(nav.urls.len(), 3);
        assert_eq!(nav.index, 2);

        let (target, url) = nav.neighbour(true).unwrap();
        assert_eq!(url, "https://b.test/");
        nav.set_pending(target);
        nav.commit("https://b.test/");
        assert_eq!(nav.index, 1);
        assert_eq!(nav.neighbour(false).unwrap().1, "https://c.test/");

        // A pending move that lands elsewhere (redirect) is a new entry and
        // drops the forward branch.
        nav.set_pending(0);
        nav.commit("https://d.test/");
        assert_eq!(
            nav.urls,
            ["https://a.test/", "https://b.test/", "https://d.test/"]
        );
        assert_eq!(nav.index, 2);
        assert!(nav.neighbour(false).is_none());
    }

    #[test]
    fn tab_store_round_trips_regular_tabs() {
        let record = TabRecord {
            id: "b7".to_string(),
            url: "https://a.test/".to_string(),
            title: "A".to_string(),
            read_only: false,
            created_ms: 1,
            ttl_ms: Some(5_000),
            history: vec![HistoryVisit {
                url: "https://a.test/".to_string(),
                title: "A".to_string(),
                timestamp: 1,
            }],
            nav: NavStack {
                urls: vec!["https://a.test/".to_string()],
                index: 0,
                pending: None,
            },
        };
        let json = serde_json::to_string(&TabStore {
            next_slot: 7,
            tabs: vec![record],
        })
        .unwrap();
        let back: TabStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.next_slot, 7);
        assert_eq!(back.tabs[0].id, "b7");
        assert_eq!(back.tabs[0].ttl_ms, Some(5_000));
        assert_eq!(back.tabs[0].nav.urls, ["https://a.test/"]);
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::http_fallback_url;

    #[test]
    fn https_on_a_local_host_falls_back_to_http_only_for_tls_failures() {
        assert_eq!(
            http_fallback_url("https://localhost:3000/app", "net::ERR_SSL_PROTOCOL_ERROR"),
            Some("http://localhost:3000/app".to_string())
        );
        assert_eq!(
            http_fallback_url("https://127.0.0.1/", "net::ERR_CERT_AUTHORITY_INVALID"),
            Some("http://127.0.0.1/".to_string())
        );
        assert_eq!(
            http_fallback_url("https://app.localhost:8443/", "net::ERR_SSL_PROTOCOL_ERROR"),
            Some("http://app.localhost:8443/".to_string())
        );
        assert!(http_fallback_url("https://example.com/", "net::ERR_SSL_PROTOCOL_ERROR").is_none());
        assert!(
            http_fallback_url("https://localhost:3000/", "net::ERR_NAME_NOT_RESOLVED").is_none()
        );
        assert!(
            http_fallback_url("http://localhost:3000/", "net::ERR_SSL_PROTOCOL_ERROR").is_none()
        );
    }
}
