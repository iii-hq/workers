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
use chromiumoxide::cdp::browser_protocol::overlay;
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

pub struct Session {
    pub id: String,
    pub headless: bool,
    pub created_ms: i64,
    browser: tokio::sync::Mutex<Browser>,
    pub page: Page,
    pub console: Arc<Mutex<RingBuffer<ConsoleEntry>>>,
    pub network: Arc<Mutex<RingBuffer<NetworkEntry>>>,
    seq: Arc<AtomicU64>,
    last_used_ms: AtomicU64,
    /// Snapshot/pick refs (`e1`, `p3`, …) → CDP backend node ids. Cleared on
    /// navigation — backend ids do not survive a document swap.
    pub refs: Mutex<HashMap<String, i64>>,
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

    pub fn store_refs(&self, map: HashMap<String, i64>) {
        *self.refs.lock().unwrap_or_else(|p| p.into_inner()) = map;
    }

    pub fn add_ref(&self, r: String, backend_node_id: i64) {
        self.refs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(r, backend_node_id);
    }

    fn clear_refs(&self) {
        self.refs.lock().unwrap_or_else(|p| p.into_inner()).clear();
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

    /// Close the page + Chromium process and abort the event pumps.
    /// Idempotent at the caller level: `Sessions::stop` removes the session
    /// from the map first, so a second stop never reaches here.
    pub async fn shutdown(&self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            task.abort();
        }
        let mut browser = self.browser.lock().await;
        if browser.close().await.is_err() {
            if let Some(Err(e)) = browser.kill().await {
                tracing::warn!(session_id = %self.id, error = %e, "browser kill failed");
            }
        }
        let _ = browser.wait().await;
    }
}

/// The live session table plus everything a pump task needs.
pub struct Sessions {
    map: Mutex<HashMap<String, Arc<Session>>>,
    counter: AtomicU64,
    pub config: SharedConfig,
    pub emitter: Arc<Emitter>,
}

impl Sessions {
    pub fn new(config: SharedConfig, emitter: Arc<Emitter>) -> Arc<Self> {
        Arc::new(Self {
            map: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
            config,
            emitter,
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
        let browser_config = build_browser_config(&cfg, headless)?;

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

        let id = format!("b{}", self.counter.fetch_add(1, Ordering::Relaxed) + 1);
        let now = now_ms();
        let session = Arc::new(Session {
            id: id.clone(),
            headless,
            created_ms: now,
            browser: tokio::sync::Mutex::new(browser),
            page: page.clone(),
            console: Arc::new(Mutex::new(RingBuffer::new(cfg.console_buffer as usize))),
            network: Arc::new(Mutex::new(RingBuffer::new(cfg.network_buffer as usize))),
            seq: Arc::new(AtomicU64::new(1)),
            last_used_ms: AtomicU64::new(now as u64),
            refs: Mutex::new(HashMap::new()),
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

    /// Remove + shut down. Returns whether the session was running —
    /// stopping an unknown id succeeds (delete semantics: the caller wants
    /// it gone, and it is).
    pub async fn stop(&self, id: &str, reason: &str) -> bool {
        let session = self.lock().remove(id);
        match session {
            Some(session) => {
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

fn build_browser_config(cfg: &WorkerConfig, headless: bool) -> Result<BrowserConfig, String> {
    let mut builder = BrowserConfig::builder()
        .window_size(cfg.viewport_width, cfg.viewport_height)
        .viewport(chromiumoxide::handler::viewport::Viewport {
            width: cfg.viewport_width,
            height: cfg.viewport_height,
            ..Default::default()
        });
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
                s.network
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(entry);
            }
        }));
    }

    if let Ok(mut events) = page
        .event_listener::<cdp_network::EventLoadingFailed>()
        .await
    {
        let s = session.clone();
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
                s.network
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(entry);
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

    // element picked in inspect mode (browser::pick::start)
    if let Ok(mut events) = page
        .event_listener::<overlay::EventInspectNodeRequested>()
        .await
    {
        let s = session.clone();
        let sx = sessions.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let backend_node_id = *event.backend_node_id.inner();
                if let Err(e) = handle_pick(&sx, &s, backend_node_id).await {
                    tracing::warn!(session_id = %s.id, error = %e, "pick resolution failed");
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

/// Resolve the picked node into a `PickedElement`, register a ref for it,
/// emit `browser::picked`, and leave inspect mode.
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

    // one pick per pick::start: leave inspect mode after the click
    let _ = page
        .execute(
            overlay::SetInspectModeParams::builder()
                .mode(overlay::InspectMode::None)
                .build()
                .map_err(|e| format!("SetInspectMode: {e}"))?,
        )
        .await;
    session.touch();
    Ok(())
}

/// Bounds from a CDP content quad `[x1,y1,x2,y2,x3,y3,x4,y4]`.
fn quad_bounds(quad: &dom::Quad) -> Bounds {
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
