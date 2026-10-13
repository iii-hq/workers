//! Private persistent-session registry for `browser::session-*`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};

use crate::config::WorkerConfig;
#[cfg(feature = "scrapling-compat")]
use crate::scrapling::fetch::CompatSession;
use crate::scrapling::fetch::HttpMode;
use crate::scrapling::raw_browser::{RawBrowser, RawBrowserOptions};
use crate::session::now_ms;

pub(crate) type Job = Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 'static>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionType {
    Http,
    Dynamic,
    Stealthy,
}

impl SessionType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Dynamic => "dynamic",
            Self::Stealthy => "stealthy",
        }
    }
}

pub fn parse_type(payload: &Value) -> Result<SessionType, String> {
    let Some(value) = payload.get("type") else {
        return Ok(SessionType::Http);
    };
    match value.as_str() {
        Some("http") => Ok(SessionType::Http),
        Some("dynamic") => Ok(SessionType::Dynamic),
        Some("stealthy") => Ok(SessionType::Stealthy),
        _ => Err(format!(
            "unknown session type: {} (use http|dynamic|stealthy)",
            python_repr(value)
        )),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::String(value) => format!("'{value}'"),
        other => other.to_string(),
    }
}

#[derive(Debug)]
pub struct HttpBackend {
    pub jar: Arc<reqwest::cookie::Jar>,
    pub mode: HttpMode,
    #[cfg(feature = "scrapling-compat")]
    pub(crate) compat: Option<CompatSession>,
    constructor: Value,
}

impl HttpBackend {
    #[cfg(test)]
    fn new(payload: &Value) -> Self {
        Self::new_for_mode(payload, HttpMode::Safe).expect("safe HTTP backend is infallible")
    }

    fn new_for_mode(payload: &Value, mode: HttpMode) -> Result<Self, String> {
        #[cfg(not(feature = "scrapling-compat"))]
        if mode == HttpMode::Compat {
            return Err(
                "browser::fetch compat HTTP engine is not compiled into this binary".to_string(),
            );
        }
        Ok(Self {
            jar: Arc::new(reqwest::cookie::Jar::default()),
            mode,
            #[cfg(feature = "scrapling-compat")]
            compat: (mode == HttpMode::Compat)
                .then(CompatSession::new)
                .transpose()?,
            constructor: constructor_config(payload),
        })
    }

    pub fn request(&self, payload: &Value) -> Value {
        merge_request(&self.constructor, payload)
    }
}

pub struct BrowserBackend {
    pub browser: Arc<RawBrowser>,
    constructor: Value,
    pub stealth: bool,
    pub security_mode: crate::config::SecurityMode,
}

impl BrowserBackend {
    pub fn request(&self, payload: &Value) -> Value {
        merge_browser_request(&self.constructor, payload)
    }
}

enum Backend {
    Http(Arc<HttpBackend>),
    Browser(Arc<BrowserBackend>),
}

enum Command {
    Run {
        job: Job,
        response: oneshot::Sender<Result<Value, String>>,
    },
    Close {
        response: oneshot::Sender<()>,
    },
}

struct Entry {
    id: String,
    session_type: SessionType,
    created_ms: i64,
    last_used_ms: AtomicI64,
    compat_only: bool,
    backend: Backend,
    commands: mpsc::UnboundedSender<Command>,
}

impl Entry {
    fn info(&self, observed_ms: i64) -> Value {
        let last = self.last_used_ms.load(Ordering::Relaxed);
        let idle = (observed_ms - last).max(0) as f64 / 1000.0;
        json!({
            "session_id": self.id,
            "type": self.session_type.as_str(),
            "created_at": self.created_ms as f64 / 1000.0,
            "last_used": last as f64 / 1000.0,
            "idle_s": (idle * 10.0).round() / 10.0,
        })
    }
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Arc<Entry>>,
    insertion_order: Vec<String>,
    pending: usize,
}

pub struct Registry {
    /// Shared with each session's actor, which removes its own entry when
    /// its idle deadline passes.
    state: Arc<Mutex<State>>,
    max_sessions: usize,
    /// None = sessions never expire for idleness.
    idle_timeout: Option<Duration>,
}

impl Registry {
    pub fn new(max_sessions: u64, idle_timeout_s: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            max_sessions: usize::try_from(max_sessions).unwrap_or(usize::MAX),
            idle_timeout: (idle_timeout_s > 0).then(|| Duration::from_secs(idle_timeout_s)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn open_with<F>(
        &self,
        session_type: SessionType,
        compat_only: bool,
        construct: F,
    ) -> Result<Value, String>
    where
        F: Future<Output = Result<Backend, String>>,
    {
        {
            let mut state = self.lock();
            if state.entries.len() + state.pending >= self.max_sessions {
                return Err(format!(
                    "session limit reached ({}); close one first",
                    self.max_sessions
                ));
            }
            state.pending += 1;
        }

        // Decrement on EVERY exit — error, panic or cancellation. A missed
        // decrement leaks a slot forever and eventually bricks session-open.
        struct PendingGuard<'a>(&'a Registry);
        impl Drop for PendingGuard<'_> {
            fn drop(&mut self) {
                self.0.lock().pending -= 1;
            }
        }
        let pending = PendingGuard(self);
        let backend = construct.await;
        drop(pending);
        let mut state = self.lock();
        let backend = backend?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let now = now_ms();
        let (commands, receiver) = mpsc::unbounded_channel();
        let entry = Arc::new(Entry {
            id: id.clone(),
            session_type,
            created_ms: now,
            last_used_ms: AtomicI64::new(now),
            compat_only,
            backend,
            commands,
        });
        tokio::spawn(run_actor(
            entry.clone(),
            receiver,
            Arc::downgrade(&self.state),
            self.idle_timeout,
        ));
        state.insertion_order.push(id.clone());
        state.entries.insert(id.clone(), entry);
        Ok(json!({"session_id": id, "type": session_type.as_str()}))
    }

    pub async fn open_http(
        &self,
        payload: &Value,
        compat_only: bool,
        mode: HttpMode,
    ) -> Result<Value, String> {
        let payload = payload.clone();
        self.open_with(SessionType::Http, compat_only, async move {
            Ok(Backend::Http(Arc::new(HttpBackend::new_for_mode(
                &payload, mode,
            )?)))
        })
        .await
    }

    pub async fn open_browser(
        &self,
        session_type: SessionType,
        payload: &Value,
        compat_only: bool,
        config: Arc<WorkerConfig>,
    ) -> Result<Value, String> {
        let constructor = browser_constructor_config(payload);
        let payload = payload.clone();
        let stealth = session_type == SessionType::Stealthy;
        self.open_with(session_type, compat_only, async move {
            let mut options = RawBrowserOptions::from_payload(&payload)?;
            options.clamp_durations(config.max_timeout_ms);
            let browser = RawBrowser::start(&config, &options, stealth, true).await?;
            Ok(Backend::Browser(Arc::new(BrowserBackend {
                browser: Arc::new(browser),
                constructor,
                stealth,
                security_mode: config.scrapling.security_mode,
            })))
        })
        .await
    }

    pub fn http_backend(&self, id: &str) -> Result<Arc<HttpBackend>, String> {
        let state = self.lock();
        let entry = state
            .entries
            .get(id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        match &entry.backend {
            Backend::Http(backend) => Ok(backend.clone()),
            Backend::Browser(_) => Err(format!("session {id} is not an HTTP session")),
        }
    }

    pub fn browser_backend(&self, id: &str) -> Result<Arc<BrowserBackend>, String> {
        let state = self.lock();
        let entry = state
            .entries
            .get(id)
            .ok_or_else(|| format!("unknown session: {id}"))?;
        match &entry.backend {
            Backend::Browser(backend) => Ok(backend.clone()),
            Backend::Http(_) => Err(format!("session {id} is not a browser session")),
        }
    }

    pub fn session_type(&self, id: &str) -> Result<SessionType, String> {
        self.lock()
            .entries
            .get(id)
            .map(|entry| entry.session_type)
            .ok_or_else(|| format!("unknown session: {id}"))
    }

    pub fn uses_compat_only_options(&self, id: &str) -> Result<bool, String> {
        self.lock()
            .entries
            .get(id)
            .map(|entry| entry.compat_only)
            .ok_or_else(|| format!("unknown session: {id}"))
    }

    pub async fn run(&self, id: &str, job: Job) -> Result<Value, String> {
        let response = {
            let state = self.lock();
            let entry = state
                .entries
                .get(id)
                .ok_or_else(|| format!("unknown session: {id}"))?;
            let (send, receive) = oneshot::channel();
            entry
                .commands
                .send(Command::Run {
                    job,
                    response: send,
                })
                .map_err(|_| format!("unknown session: {id}"))?;
            receive
        };
        response
            .await
            .map_err(|_| format!("unknown session: {id}"))?
    }

    pub async fn close(&self, id: &str) -> Value {
        let response = {
            let mut state = self.lock();
            let Some(entry) = state.entries.remove(id) else {
                return json!({"closed": false});
            };
            state.insertion_order.retain(|existing| existing != id);
            let (send, receive) = oneshot::channel();
            let _ = entry.commands.send(Command::Close { response: send });
            receive
        };
        let _ = response.await;
        json!({"closed": true})
    }

    pub async fn close_all(&self) {
        let ids = {
            let state = self.lock();
            state
                .insertion_order
                .iter()
                .filter(|id| state.entries.contains_key(*id))
                .cloned()
                .collect::<Vec<_>>()
        };
        futures::future::join_all(ids.iter().map(|id| self.close(id))).await;
    }

    pub fn list(&self, type_filter: Option<&Value>) -> Value {
        let now = now_ms();
        let state = self.lock();
        let sessions = state
            .insertion_order
            .iter()
            .filter_map(|id| state.entries.get(id))
            .filter(|entry| match type_filter {
                None => true,
                Some(Value::String(wanted)) => entry.session_type.as_str() == wanted,
                Some(_) => false,
            })
            .map(|entry| entry.info(now))
            .collect::<Vec<_>>();
        json!({"sessions": sessions})
    }
}

/// Remove `entry` from the registry if it is still listed; true when this
/// call removed it.
fn unlist(state: &Weak<Mutex<State>>, entry: &Arc<Entry>) -> bool {
    let Some(state) = state.upgrade() else {
        return false;
    };
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let listed = state
        .entries
        .get(&entry.id)
        .is_some_and(|listed| Arc::ptr_eq(listed, entry));
    if listed {
        state.entries.remove(&entry.id);
        state
            .insertion_order
            .retain(|existing| existing != &entry.id);
    }
    listed
}

/// One session's actor: runs its jobs in order and closes it on `Close`.
/// It also owns the idle deadline: `idle` after the last job finished (or
/// the open) the session unlists itself and closes, after any job already
/// queued. Nothing wakes before that deadline unless a command arrives.
async fn run_actor(
    entry: Arc<Entry>,
    mut receiver: mpsc::UnboundedReceiver<Command>,
    state: Weak<Mutex<State>>,
    idle: Option<Duration>,
) {
    let mut idle_due = idle.map(|idle| tokio::time::Instant::now() + idle);
    loop {
        let command = tokio::select! {
            biased;
            command = receiver.recv() => match command {
                Some(command) => command,
                None => return,
            },
            _ = tokio::time::sleep_until(idle_due.unwrap_or_else(tokio::time::Instant::now)),
                if idle_due.is_some() =>
            {
                idle_due = None;
                if unlist(&state, &entry) {
                    tracing::info!(session = %entry.id, "scrapling session reaped (idle)");
                    // Queued behind any job accepted before the unlist, the
                    // way an explicit close is.
                    let (response, _) = oneshot::channel();
                    let _ = entry.commands.send(Command::Close { response });
                }
                continue;
            }
        };
        match command {
            Command::Run { job, response } => {
                entry.last_used_ms.store(now_ms(), Ordering::Relaxed);
                // Jobs are now bounded from below (fetch timeouts clamped to
                // max_timeout_ms, every CDP command has a hard ceiling), so a
                // job can no longer hang forever and wedge the Close queued
                // behind it — the root cause the FIFO actor used to expose.
                let result = job.await;
                entry.last_used_ms.store(now_ms(), Ordering::Relaxed);
                if idle_due.is_some() {
                    idle_due = idle.map(|idle| tokio::time::Instant::now() + idle);
                }
                let _ = response.send(result);
            }
            Command::Close { response } => {
                if let Backend::Browser(backend) = &entry.backend {
                    // Bounded: a wedged CDP connection must not make Close
                    // (or worker shutdown) wait forever; dropping the client
                    // kills the Chromium child regardless.
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        backend.browser.shutdown(),
                    )
                    .await;
                }
                drop(receiver);
                drop(entry);
                let _ = response.send(());
                return;
            }
        }
    }
}

const HTTP_CONSTRUCTOR_KEYS: &[&str] = &[
    "impersonate",
    "http3",
    "stealthy_headers",
    "proxy",
    "proxies",
    "proxy_auth",
    "timeout",
    "headers",
    "retries",
    "retry_delay",
    "follow_redirects",
    "max_redirects",
    "verify",
];

const BROWSER_CONSTRUCTOR_KEYS: &[&str] = &[
    "headless",
    "network_idle",
    "load_dom",
    "timeout",
    "wait",
    "wait_selector",
    "wait_selector_state",
    "disable_resources",
    "proxy",
    "useragent",
    "cookies",
    "google_search",
    "block_ads",
    "blocked_domains",
    "real_chrome",
    "cdp_url",
    "capture_xhr",
    "locale",
    "timezone_id",
    "extra_headers",
    "dns_over_https",
    "retries",
    "retry_delay",
    "extra_flags",
    "max_pages",
    "solve_cloudflare",
    "block_webrtc",
    "hide_canvas",
    "allow_webgl",
];

const BROWSER_FETCH_KEYS: &[&str] = &[
    "google_search",
    "timeout",
    "wait",
    "wait_selector",
    "wait_selector_state",
    "disable_resources",
    "extra_headers",
    "network_idle",
    "load_dom",
    "blocked_domains",
    "solve_cloudflare",
    "proxy",
];

fn constructor_config(payload: &Value) -> Value {
    selected_config(payload, HTTP_CONSTRUCTOR_KEYS)
}

fn browser_constructor_config(payload: &Value) -> Value {
    selected_config(payload, BROWSER_CONSTRUCTOR_KEYS)
}

fn selected_config(payload: &Value, keys: &[&str]) -> Value {
    let mut result = Map::new();
    if let Some(values) = payload.as_object() {
        for &key in keys {
            if let Some(value) = values.get(key).filter(|value| !value.is_null()) {
                result.insert(key.to_string(), value.clone());
            }
        }
    }
    Value::Object(result)
}

fn merge_request(constructor: &Value, payload: &Value) -> Value {
    let mut result = constructor.as_object().cloned().unwrap_or_default();
    if let Some(values) = payload.as_object() {
        for (key, value) in values {
            if key != "session_id" && key != "type" && !value.is_null() {
                result.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(result)
}

fn merge_browser_request(constructor: &Value, payload: &Value) -> Value {
    let mut result = constructor.as_object().cloned().unwrap_or_default();
    if let Some(values) = payload.as_object() {
        for (key, value) in values {
            if key != "session_id"
                && key != "type"
                && !value.is_null()
                && !(key == "proxy" && value.as_str() == Some(""))
                && (!BROWSER_CONSTRUCTOR_KEYS.contains(&key.as_str())
                    || BROWSER_FETCH_KEYS.contains(&key.as_str()))
            {
                result.insert(key.clone(), value.clone());
            }
        }
        if values
            .get("headers")
            .and_then(Value::as_object)
            .is_some_and(|headers| !headers.is_empty())
            && !values.contains_key("extra_headers")
        {
            result.insert("extra_headers".to_string(), values["headers"].clone());
        }
    }
    Value::Object(result)
}

/// Every option that safe mode refuses somewhere must be listed here, or a
/// session opens fine and only errors (or is silently ignored) at first
/// fetch. Keep in sync with the safe-mode rejects in
/// `HttpOptions::from_payload_for_mode` (HTTP tier) and
/// `RawBrowserOptions::validate_policy` (browser tier).
pub fn uses_compat_only_options(payload: &Value) -> bool {
    payload.get("verify") == Some(&Value::Bool(false))
        || ["proxy", "cdp_url"].iter().any(|key| {
            payload
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|v| !v.is_empty())
        })
        || payload.get("proxy_auth").is_some_and(|v| !v.is_null())
        || payload
            .get("proxies")
            .and_then(Value::as_object)
            .is_some_and(|v| !v.is_empty())
        || payload
            .get("extra_flags")
            .and_then(Value::as_array)
            .is_some_and(|v| !v.is_empty())
        || payload.get("real_chrome") == Some(&Value::Bool(true))
        || payload.get("dns_over_https") == Some(&Value::Bool(true))
        || payload.get("stealthy_headers") == Some(&Value::Bool(true))
        || payload.get("http3") == Some(&Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::Duration;

    use super::*;

    fn id(response: &Value) -> &str {
        response["session_id"].as_str().unwrap()
    }

    #[tokio::test]
    async fn ids_are_uuid4_lowercase_hex_and_interactive_ids_are_foreign() {
        let registry = Registry::new(8, 900);
        let opened = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let sid = id(&opened);
        assert_eq!(sid.len(), 32);
        assert!(sid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_eq!(&sid[12..13], "4");
        assert_eq!(
            registry
                .run("b1", Box::pin(async { Ok(Value::Null) }))
                .await
                .unwrap_err(),
            "unknown session: b1"
        );
    }

    #[tokio::test]
    async fn active_plus_pending_reserves_capacity_and_rolls_back_failure() {
        let registry = Arc::new(Registry::new(1, 900));
        let gate = Arc::new(tokio::sync::Notify::new());
        let opening = {
            let registry = registry.clone();
            let gate = gate.clone();
            tokio::spawn(async move {
                registry
                    .open_with(SessionType::Http, false, async move {
                        gate.notified().await;
                        Err("constructor failed".to_string())
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            registry
                .open_http(&json!({}), false, HttpMode::Safe)
                .await
                .unwrap_err(),
            "session limit reached (1); close one first"
        );
        gate.notify_one();
        assert_eq!(opening.await.unwrap().unwrap_err(), "constructor failed");
        registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn same_session_is_fifo_close_queues_and_different_sessions_overlap() {
        let registry = Arc::new(Registry::new(2, 900));
        let first = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let second = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));
        let run = |sid: String, marker: usize| {
            let registry = registry.clone();
            let active = active.clone();
            let peak = peak.clone();
            let order = order.clone();
            tokio::spawn(async move {
                registry
                    .run(
                        &sid,
                        Box::pin(async move {
                            let now = active.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                            peak.fetch_max(now, AtomicOrdering::SeqCst);
                            order.lock().unwrap().push(marker);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            active.fetch_sub(1, AtomicOrdering::SeqCst);
                            Ok(json!(marker))
                        }),
                    )
                    .await
            })
        };
        let a = run(id(&first).to_string(), 1);
        tokio::task::yield_now().await;
        let b = run(id(&first).to_string(), 2);
        let c = run(id(&second).to_string(), 3);
        tokio::task::yield_now().await;
        let close = {
            let registry = registry.clone();
            let sid = id(&first).to_string();
            tokio::spawn(async move { registry.close(&sid).await })
        };
        assert_eq!(a.await.unwrap().unwrap(), json!(1));
        assert_eq!(b.await.unwrap().unwrap(), json!(2));
        assert_eq!(c.await.unwrap().unwrap(), json!(3));
        assert_eq!(close.await.unwrap(), json!({"closed": true}));
        assert!(peak.load(AtomicOrdering::SeqCst) >= 2);
        let order = order.lock().unwrap();
        assert!(order.iter().position(|v| *v == 1) < order.iter().position(|v| *v == 2));
    }

    #[tokio::test]
    async fn list_preserves_insertion_order_and_exact_type_filter() {
        let registry = Registry::new(3, 900);
        let http = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let second = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let listed = registry.list(None);
        assert_eq!(listed["sessions"][0]["session_id"], http["session_id"]);
        assert_eq!(listed["sessions"][1]["session_id"], second["session_id"]);
        assert_eq!(
            registry.list(Some(&json!("stealthy"))),
            json!({"sessions": []})
        );
        assert_eq!(registry.list(Some(&json!(3))), json!({"sessions": []}));
    }

    #[tokio::test]
    async fn private_browser_backend_opens_lists_and_closes() {
        let executable = std::env::var_os("SCRAPLING_CHROMIUM_EXECUTABLE")
            .map(std::path::PathBuf::from)
            .or_else(|| crate::functions::doctor::detect_executable(&WorkerConfig::default()));
        let Some(executable) = executable.filter(|executable| {
            crate::functions::doctor::chromium_version(executable).is_some_and(|version| {
                version.split_whitespace().any(|part| {
                    crate::scrapling::raw_browser::CERTIFIED_CHROME_VERSIONS.contains(&part)
                })
            })
        }) else {
            #[cfg(feature = "scrapling-compat")]
            panic!("certified tests require SCRAPLING_CHROMIUM_EXECUTABLE=Chrome-148");
            #[cfg(not(feature = "scrapling-compat"))]
            return;
        };
        let mut config = WorkerConfig::default();
        config.scrapling.chromium_executable = executable.display().to_string();
        let registry = Registry::new(2, 900);
        let http = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let opened = registry
            .open_browser(
                SessionType::Dynamic,
                &json!({"headless": true}),
                false,
                Arc::new(config),
            )
            .await
            .unwrap();
        let sid = id(&opened).to_string();
        assert_eq!(opened["type"], "dynamic");
        assert_eq!(
            registry.list(None)["sessions"][0]["session_id"],
            http["session_id"]
        );
        assert_eq!(registry.list(None)["sessions"][1]["session_id"], sid);
        assert_eq!(
            registry.list(Some(&json!("dynamic")))["sessions"][0]["session_id"],
            sid
        );
        assert_eq!(
            registry
                .open_http(&json!({}), false, HttpMode::Safe)
                .await
                .unwrap_err(),
            "session limit reached (2); close one first"
        );
        assert_eq!(registry.close(&sid).await, json!({"closed": true}));
        assert_eq!(registry.close(id(&http)).await, json!({"closed": true}));
    }

    #[test]
    fn constructor_state_is_allowlisted_and_null_request_values_do_not_override() {
        let backend =
            HttpBackend::new(&json!({"type":"http", "timeout":5, "json":{"no":"constructor"}}));
        let merged = backend.request(&json!({"session_id":"id", "timeout":null, "url":"u"}));
        assert_eq!(merged, json!({"timeout":5, "url":"u"}));
    }

    #[test]
    fn browser_fetch_overrides_only_request_state() {
        let constructor = browser_constructor_config(&json!({
            "useragent": "constructor-agent",
            "wait_selector": "#constructor",
            "extra_headers": {"x-constructor": "yes"},
            "cookies": [{"name": "a", "value": "b"}]
        }));
        let merged = merge_browser_request(
            &constructor,
            &json!({
                "session_id": "id",
                "url": "https://example.test",
                "useragent": "ignored-request-agent",
                "cookies": [],
                "wait_selector": "#request",
                "headers": {"authorization": "Bearer token"},
                "proxy": "http://request-proxy:8080"
            }),
        );
        assert_eq!(merged["useragent"], "constructor-agent");
        assert_eq!(merged["cookies"], json!([{"name": "a", "value": "b"}]));
        assert_eq!(merged["wait_selector"], "#request");
        assert_eq!(
            merged["extra_headers"],
            json!({"authorization": "Bearer token"})
        );
        assert_eq!(merged["proxy"], "http://request-proxy:8080");

        let constructor = browser_constructor_config(&json!({
            "proxy": "http://constructor-proxy:8080"
        }));
        assert_eq!(
            merge_browser_request(&constructor, &json!({"proxy": ""}))["proxy"],
            "http://constructor-proxy:8080"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_reap_removes_metadata_and_backend_together() {
        let registry = Registry::new(1, 1);
        let opened = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let sid = id(&opened).to_string();
        // Its own deadline, one second after the open; nothing sweeps.
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert_eq!(registry.list(None), json!({"sessions": []}));
        assert_eq!(
            registry.http_backend(&sid).unwrap_err(),
            format!("unknown session: {sid}")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_rearms_the_idle_deadline() {
        let registry = Registry::new(1, 1);
        let opened = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        let sid = id(&opened).to_string();
        tokio::time::sleep(Duration::from_millis(600)).await;
        registry
            .run(&sid, Box::pin(async { Ok(Value::Null) }))
            .await
            .unwrap();
        // 1.2s after the open but 0.6s after the job: still listed.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(registry.http_backend(&sid).is_ok());
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(registry.http_backend(&sid).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_timeout_zero_never_reaps() {
        let registry = Registry::new(1, 0);
        let opened = registry
            .open_http(&json!({}), false, HttpMode::Safe)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
        assert!(registry.http_backend(id(&opened)).is_ok());
    }

    #[tokio::test]
    async fn compat_metadata_and_close_are_exact_and_idempotent() {
        let registry = Registry::new(1, 900);
        let opened = registry
            .open_http(&json!({"verify": false}), true, HttpMode::Safe)
            .await
            .unwrap();
        let sid = id(&opened).to_string();
        assert!(registry.uses_compat_only_options(&sid).unwrap());
        assert_eq!(registry.close(&sid).await, json!({"closed": true}));
        assert_eq!(registry.close(&sid).await, json!({"closed": false}));
    }

    #[cfg(feature = "scrapling-compat")]
    #[tokio::test]
    async fn compat_http_transport_is_snapshotted_at_open() {
        let registry = Registry::new(1, 900);
        let opened = registry
            .open_http(&json!({}), true, HttpMode::Compat)
            .await
            .unwrap();
        let sid = id(&opened);
        let backend = registry.http_backend(sid).unwrap();
        assert_eq!(backend.mode, HttpMode::Compat);
        assert!(backend.compat.is_some());
        assert_eq!(registry.close(sid).await, json!({"closed": true}));
    }

    #[test]
    fn response_and_error_text_match_the_frozen_wrapper() {
        assert_eq!(parse_type(&json!({})).unwrap(), SessionType::Http);
        assert_eq!(
            parse_type(&json!({"type":"stealthy"})).unwrap(),
            SessionType::Stealthy
        );
        assert_eq!(
            parse_type(&json!({"type":"carrier-pigeon"})).unwrap_err(),
            "unknown session type: 'carrier-pigeon' (use http|dynamic|stealthy)"
        );
        assert_eq!(
            parse_type(&json!({"type":null})).unwrap_err(),
            "unknown session type: None (use http|dynamic|stealthy)"
        );
    }

    #[test]
    fn every_unsafe_constructor_shape_marks_the_session_compat_only() {
        for payload in [
            json!({"verify": false}),
            json!({"proxy": "http://proxy"}),
            json!({"proxies": {"https": "http://proxy"}}),
            json!({"proxy_auth": ["user", "pass"]}),
            json!({"cdp_url": "ws://browser"}),
            json!({"extra_flags": ["--no-sandbox"]}),
            json!({"real_chrome": true}),
        ] {
            assert!(uses_compat_only_options(&payload), "{payload}");
        }
        assert!(!uses_compat_only_options(&json!({"extra_flags": []})));
    }
}
