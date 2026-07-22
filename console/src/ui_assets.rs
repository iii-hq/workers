//! Injectable console UI — the `console:*` trigger types and asset registry.
//!
//! Implements the injection protocol from
//! `iii/tech-specs/2026-07-17-injectable-ui/injection-protocol.md`:
//!
//! - Workers register `console:script` / `console:style` triggers whose
//!   `config.path` is the asset's identity; the trigger's `function_id`
//!   names the *content function* the console invokes to fetch source.
//! - Console tabs register `console:assets` triggers — subscriptions whose
//!   `function_id` is the per-tab handler the console pushes
//!   `sync`/`set`/`delete` events to.
//! - "Same path ⇒ override" is console-worker policy: a path-keyed
//!   last-writer-wins registry, with `engine::unregister_trigger` pruning
//!   the superseded engine row so at most one live trigger per path exists
//!   in steady state.
//!
//! The SDK delivers each incoming register/unregister callback on its own
//! spawned task, so all `console:*` events — assets *and* subscriptions —
//! drain through a single-consumer mpsc queue; the content fetch runs
//! inside the serialized section. (Enqueue order can in principle diverge
//! from WS-arrival order across the SDK's per-message tasks; the window is
//! sub-millisecond and the hash dedupe + unknown-id no-ops absorb it.)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

pub const SCRIPT_TYPE: &str = "console:script";
pub const STYLE_TYPE: &str = "console:style";
pub const ASSETS_TYPE: &str = "console:assets";

/// Max bytes per fetched asset. Multi-MiB assets are a bundling smell; the
/// cap keeps a misbehaving worker from ballooning console memory.
const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;

/// Content-fetch budget at registration time: 2 attempts × 3s + 250ms
/// backoff ≈ 6.5s worst case — safely inside the engine's 10s registration
/// ack window (`REGISTRATION_ACK_TIMEOUT`).
const FETCH_ATTEMPTS: u32 = 2;
const FETCH_TIMEOUT_MS: u64 = 3_000;
const FETCH_BACKOFF_MS: u64 = 250;

/// Trigger config schema for both asset types (`additionalProperties`
/// enforced console-side; the engine treats trigger config schemas as
/// advisory).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetTriggerConfig {
    /// Asset identity. Convention: `<worker>/<name>.<ext>`. Re-registering
    /// a path overrides it.
    pub path: String,
}

/// `console:assets` subscriptions carry no config; every subscriber gets
/// every event.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionTriggerConfig {}

/// Short asset kind — trigger type ids are `console:*`, kinds stay
/// `script`/`style` in push payloads and the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Script,
    Style,
}

impl AssetKind {
    pub fn content_type(self) -> &'static str {
        match self {
            AssetKind::Script => "text/javascript; charset=utf-8",
            AssetKind::Style => "text/css; charset=utf-8",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            AssetKind::Script => ".js",
            AssetKind::Style => ".css",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            AssetKind::Script => "script",
            AssetKind::Style => "style",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub trigger_id: String,
    pub kind: AssetKind,
    /// First 16 hex chars of sha256(content) — the version that travels in
    /// pushes, the manifest, and `?v=` cache busting.
    pub hash: String,
    pub content: String,
    pub content_type: String,
    /// Style-lint findings (warn-only). Empty for scripts and clean styles.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ManifestAsset {
    pub path: String,
    pub kind: AssetKind,
    pub hash: String,
    /// Best-effort registrant attribution. Not resolved in this build —
    /// always `null` (the `<worker>/` path prefix is the human-readable
    /// attribution).
    pub worker: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct RegistryInner {
    /// path → asset (content pinned to the advertised hash).
    by_path: HashMap<String, Asset>,
    /// trigger id → path. The engine's `UnregisterTrigger` carries only the
    /// id, hence the second key.
    by_id: HashMap<String, String>,
    /// `console:assets` subscription id → per-tab handler function id.
    subscribers: HashMap<String, String>,
}

/// In-memory asset registry + subscriber set. Rebuilt from engine replay on
/// console restart; deliberately not persisted.
#[derive(Default)]
pub struct UiRegistry {
    inner: Mutex<RegistryInner>,
}

impl UiRegistry {
    /// Bytes + headers for `GET /ui/<path>`.
    pub fn serve(&self, path: &str) -> Option<(String, String, String)> {
        let inner = self.lock();
        let asset = inner.by_path.get(path)?;
        Some((
            asset.content.clone(),
            asset.content_type.clone(),
            asset.hash.clone(),
        ))
    }

    pub fn manifest(&self) -> Vec<ManifestAsset> {
        let inner = self.lock();
        let mut assets: Vec<ManifestAsset> = inner
            .by_path
            .iter()
            .map(|(path, a)| ManifestAsset {
                path: path.clone(),
                kind: a.kind,
                hash: a.hash.clone(),
                worker: None,
                warnings: a.warnings.clone(),
            })
            .collect();
        assets.sort_by(|a, b| a.path.cmp(&b.path));
        assets
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// What the queue consumer needs from the bus, abstracted so the override
/// algorithm is unit-testable without an engine (the state worker's
/// `Invoker` precedent).
#[async_trait]
pub trait UiBus: Send + Sync + 'static {
    /// Invoke the trigger's content function: `{path}` → `{content, content_type?}`.
    async fn fetch_content(&self, function_id: &str, path: &str) -> Result<FetchedContent, String>;
    /// Fire-and-forget push to one subscriber's per-tab handler.
    async fn push(&self, function_id: &str, payload: Value);
    /// Prune a superseded trigger row from the engine registry (idempotent).
    async fn unregister_engine_trigger(&self, trigger_id: &str);
}

#[derive(Debug, Clone)]
pub struct FetchedContent {
    pub content: String,
    pub content_type: Option<String>,
}

struct SdkBus {
    iii: Arc<IIIClient>,
}

#[async_trait]
impl UiBus for SdkBus {
    async fn fetch_content(&self, function_id: &str, path: &str) -> Result<FetchedContent, String> {
        let mut last_err = String::new();
        for attempt in 1..=FETCH_ATTEMPTS {
            match self
                .iii
                .trigger(TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: json!({ "path": path }),
                    action: None,
                    timeout_ms: Some(FETCH_TIMEOUT_MS),
                })
                .await
            {
                Ok(resp) => {
                    let content = resp
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| {
                            format!(
                                "content function '{function_id}' returned no string `content` \
                                 field for path '{path}'"
                            )
                        })?;
                    let content_type = resp
                        .get("content_type")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    return Ok(FetchedContent {
                        content,
                        content_type,
                    });
                }
                Err(e) => {
                    last_err = e.to_string();
                    if attempt < FETCH_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(FETCH_BACKOFF_MS)).await;
                    }
                }
            }
        }
        Err(format!(
            "content fetch via '{function_id}' failed after {FETCH_ATTEMPTS} attempts: {last_err}"
        ))
    }

    async fn push(&self, function_id: &str, payload: Value) {
        let result = self
            .iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            })
            .await;
        if let Err(e) = result {
            // A dead tab's subscription is GC'd by the engine moments later.
            tracing::debug!(function_id, error = %e, "ui-assets push failed");
        }
    }

    async fn unregister_engine_trigger(&self, trigger_id: &str) {
        let result = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::unregister_trigger".to_string(),
                payload: json!({ "id": trigger_id }),
                action: None,
                timeout_ms: Some(FETCH_TIMEOUT_MS),
            })
            .await;
        if let Err(e) = result {
            tracing::warn!(trigger_id, error = %e, "failed to prune superseded ui trigger");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventSource {
    Script,
    Style,
    Subscription,
}

enum QueueEvent {
    Register {
        source: EventSource,
        config: TriggerConfig,
        ack: oneshot::Sender<Result<(), String>>,
    },
    Unregister {
        config: TriggerConfig,
        ack: oneshot::Sender<()>,
    },
}

/// One handler instance per trigger type; all three feed the same queue so
/// asset commits and subscription syncs are mutually ordered.
struct QueueingTriggerHandler {
    source: EventSource,
    tx: mpsc::UnboundedSender<QueueEvent>,
}

#[async_trait]
impl TriggerHandler for QueueingTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(QueueEvent::Register {
                source: self.source,
                config,
                ack: ack_tx,
            })
            .map_err(|_| Error::Handler("ui asset queue is gone".to_string()))?;
        match ack_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(msg)) => Err(Error::Handler(msg)),
            Err(_) => Err(Error::Handler("ui asset queue dropped the event".to_string())),
        }
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(QueueEvent::Unregister {
                config,
                ack: ack_tx,
            })
            .map_err(|_| Error::Handler("ui asset queue is gone".to_string()))?;
        let _ = ack_rx.await;
        Ok(())
    }
}

pub struct Processor {
    registry: Arc<UiRegistry>,
    bus: Arc<dyn UiBus>,
}

impl Processor {
    pub fn new(registry: Arc<UiRegistry>, bus: Arc<dyn UiBus>) -> Self {
        Self { registry, bus }
    }

    async fn run(self, mut rx: mpsc::UnboundedReceiver<QueueEvent>) {
        while let Some(event) = rx.recv().await {
            match event {
                QueueEvent::Register {
                    source,
                    config,
                    ack,
                } => {
                    let result = self.register(source, config).await;
                    let _ = ack.send(result);
                }
                QueueEvent::Unregister { config, ack } => {
                    self.unregister(config).await;
                    let _ = ack.send(());
                }
            }
        }
    }

    async fn register(&self, source: EventSource, config: TriggerConfig) -> Result<(), String> {
        match source {
            EventSource::Subscription => {
                self.register_subscription(config).await;
                Ok(())
            }
            EventSource::Script => self.register_asset(AssetKind::Script, config).await,
            EventSource::Style => self.register_asset(AssetKind::Style, config).await,
        }
    }

    /// Record the subscriber, then immediately push `sync` — whatever
    /// committed before the subscription is in the sync, whatever commits
    /// after arrives incrementally (same queue), so there is no seed/frame
    /// race.
    async fn register_subscription(&self, config: TriggerConfig) {
        let sync_assets: Vec<Value> = {
            let mut inner = self.registry.lock();
            inner
                .subscribers
                .insert(config.id.clone(), config.function_id.clone());
            inner
                .by_path
                .iter()
                .map(|(path, a)| {
                    json!({ "path": path, "kind": a.kind.as_str(), "hash": a.hash })
                })
                .collect()
        };
        tracing::info!(
            subscriber = %config.function_id,
            assets = sync_assets.len(),
            "console:assets subscription registered"
        );
        self.bus
            .push(
                &config.function_id,
                json!({ "event": "sync", "assets": sync_assets }),
            )
            .await;
    }

    /// The override algorithm — must be idempotent (replay re-delivers
    /// accepted bindings).
    async fn register_asset(&self, kind: AssetKind, config: TriggerConfig) -> Result<(), String> {
        let path = config
            .config
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "trigger config must carry a string `path`".to_string())?
            .to_string();
        validate_path(&path)?;
        if !path.ends_with(kind.extension()) {
            return Err(format!(
                "path '{path}' must end in '{}' for a {} asset",
                kind.extension(),
                SCRIPT_OR_STYLE(kind),
            ));
        }

        // Replay fast-path: same trigger id and the registry already holds
        // this path from that id — refetch, and if the hash is unchanged,
        // publish nothing. (The fetch is unavoidable: the hash lives in the
        // content.)
        let fetched = self
            .bus
            .fetch_content(&config.function_id, &path)
            .await?;
        if fetched.content.len() > MAX_ASSET_BYTES {
            return Err(format!(
                "asset '{path}' is {} bytes — over the {MAX_ASSET_BYTES}-byte cap; \
                 check the worker's bundling",
                fetched.content.len()
            ));
        }
        let hash = content_hash(&fetched.content);
        let warnings = match kind {
            AssetKind::Style => lint_style(&fetched.content),
            AssetKind::Script => Vec::new(),
        };
        for w in &warnings {
            tracing::warn!(path = %path, "style lint: {w}");
        }
        let asset = Asset {
            trigger_id: config.id.clone(),
            kind,
            hash: hash.clone(),
            content: fetched.content,
            content_type: fetched
                .content_type
                .unwrap_or_else(|| kind.content_type().to_string()),
            warnings,
        };

        // Commit under one lock; bus calls (prune + pushes) happen after,
        // still inside the serialized queue section.
        let mut superseded_id: Option<String> = None;
        let mut moved_from_path: Option<String> = None;
        let subscribers: Vec<String>;
        {
            let mut inner = self.registry.lock();

            if let Some(existing) = inner.by_path.get(&path) {
                if existing.trigger_id == config.id && existing.hash == hash {
                    // Replay dedupe: nothing changed, publish nothing.
                    return Ok(());
                }
                if existing.trigger_id != config.id {
                    // Supersede: last writer wins, including across workers.
                    tracing::warn!(
                        path = %path,
                        old_trigger = %existing.trigger_id,
                        new_trigger = %config.id,
                        "ui asset path re-registered; superseding previous trigger"
                    );
                    let old_id = existing.trigger_id.clone();
                    inner.by_id.remove(&old_id);
                    superseded_id = Some(old_id);
                }
            }

            // Id reuse under a different path — an implicit move. The
            // engine's row for this id already IS the new binding, so no
            // engine-side prune.
            if let Some(old_path) = inner.by_id.get(&config.id).cloned() {
                if old_path != path {
                    inner.by_path.remove(&old_path);
                    moved_from_path = Some(old_path);
                }
            }

            inner.by_id.insert(config.id.clone(), path.clone());
            inner.by_path.insert(path.clone(), asset);
            subscribers = inner.subscribers.values().cloned().collect();
        }

        if let Some(old_id) = superseded_id {
            self.bus.unregister_engine_trigger(&old_id).await;
        }
        if let Some(old_path) = moved_from_path {
            let payload =
                json!({ "event": "delete", "path": old_path, "kind": kind.as_str(), "hash": "" });
            for sub in &subscribers {
                self.bus.push(sub, payload.clone()).await;
            }
        }
        tracing::info!(path = %path, hash = %hash, kind = kind.as_str(), "ui asset published");
        let payload = json!({ "event": "set", "path": path, "kind": kind.as_str(), "hash": hash });
        for sub in &subscribers {
            self.bus.push(sub, payload.clone()).await;
        }
        Ok(())
    }

    /// The engine sends only `{id, trigger_type}` on this leg, hence the
    /// `by_id` lookup. Unknown ids are no-ops — that is exactly what a
    /// superseded trigger's own unregister (or its engine echo) looks like.
    async fn unregister(&self, config: TriggerConfig) {
        let mut removed: Option<(String, AssetKind, String)> = None;
        let subscribers: Vec<String>;
        {
            let mut inner = self.registry.lock();
            if inner.subscribers.remove(&config.id).is_some() {
                return;
            }
            if let Some(path) = inner.by_id.remove(&config.id) {
                let matches = inner
                    .by_path
                    .get(&path)
                    .is_some_and(|a| a.trigger_id == config.id);
                if matches {
                    if let Some(asset) = inner.by_path.remove(&path) {
                        removed = Some((path, asset.kind, asset.hash));
                    }
                }
            }
            subscribers = inner.subscribers.values().cloned().collect();
        }
        if let Some((path, kind, hash)) = removed {
            tracing::info!(path = %path, "ui asset removed");
            let payload = json!({
                "event": "delete", "path": path, "kind": kind.as_str(), "hash": hash
            });
            for sub in &subscribers {
                self.bus.push(sub, payload.clone()).await;
            }
        }
    }
}

#[allow(non_snake_case)]
fn SCRIPT_OR_STYLE(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Script => SCRIPT_TYPE,
        AssetKind::Style => STYLE_TYPE,
    }
}

/// Register the three `console:*` trigger types and spawn the queue
/// consumer. Must run **before** `functions::register_all` (the
/// approval-gate/memory ordering convention). Returns the registry the
/// HTTP routes and `console::ui-manifest` read from.
pub fn start(iii: &Arc<IIIClient>) -> Arc<UiRegistry> {
    let registry = Arc::new(UiRegistry::default());
    let bus = Arc::new(SdkBus { iii: iii.clone() });
    let (tx, rx) = mpsc::unbounded_channel();
    let processor = Processor::new(registry.clone(), bus);
    tokio::spawn(processor.run(rx));

    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            SCRIPT_TYPE,
            "An ESM JavaScript asset injected into the console UI. `config.path` is the \
             asset identity (re-registering a path overrides it); the trigger's \
             function_id is the content function the console fetches source from.",
            QueueingTriggerHandler {
                source: EventSource::Script,
                tx: tx.clone(),
            },
        )
        .trigger_request_format::<AssetTriggerConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            STYLE_TYPE,
            "A CSS asset injected into the console UI. Same contract as console:script \
             with a .css path.",
            QueueingTriggerHandler {
                source: EventSource::Style,
                tx: tx.clone(),
            },
        )
        .trigger_request_format::<AssetTriggerConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            ASSETS_TYPE,
            "A console tab's live-update subscription. The trigger's function_id is the \
             per-tab handler the console pushes sync/set/delete asset events to.",
            QueueingTriggerHandler {
                source: EventSource::Subscription,
                tx,
            },
        )
        .trigger_request_format::<SubscriptionTriggerConfig>(),
    );
    tracing::info!(
        trigger_types = ?[SCRIPT_TYPE, STYLE_TYPE, ASSETS_TYPE],
        "registered injectable-ui trigger types"
    );
    registry
}

pub fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..16].to_string()
}

/// Path rules from the injection protocol:
/// `^[a-z0-9][a-z0-9._-]*(/[a-z0-9][a-z0-9._-]*)*$`, no `.`/`..` segments —
/// the path becomes a URL segment under `/ui/`.
pub fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("path must not be empty".to_string());
    }
    if path.len() > 512 {
        return Err("path is over 512 characters".to_string());
    }
    if path.contains('\\') {
        return Err(format!("path '{path}' must not contain backslashes"));
    }
    if path.starts_with('/') {
        return Err(format!("path '{path}' must not start with a slash"));
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(format!("path '{path}' has an empty segment"));
        }
        if segment.chars().all(|c| c == '.') {
            return Err(format!("path '{path}' has a dot-only segment"));
        }
        let first = segment.as_bytes()[0];
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(format!(
                "path segment '{segment}' must start with a lowercase letter or digit"
            ));
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        {
            return Err(format!(
                "path segment '{segment}' has characters outside [a-z0-9._-]"
            ));
        }
    }
    Ok(())
}

/// Warn-only scan for selectors that escape the asset's `[data-iii-ui]`
/// scope. Catches the *accidental* console-wide restyle — an unlayered
/// injected rule beats the console's fully-layered CSS at equal
/// specificity. Deliberately cheap: top-level selectors only, plus
/// `@font-face` anywhere (inherently unscopable).
pub fn lint_style(content: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let stripped = strip_css_comments(content);

    if stripped.contains("@font-face") {
        warnings.push(
            "@font-face cannot be scoped under [data-iii-ui] — it applies document-wide"
                .to_string(),
        );
    }

    let mut depth: i32 = 0;
    let mut selector = String::new();
    for ch in stripped.chars() {
        match ch {
            '{' => {
                if depth == 0 {
                    let sel = selector.trim().to_string();
                    if !sel.is_empty() && !sel.starts_with('@') {
                        for part in sel.split(',') {
                            if let Some(w) = lint_selector(part.trim()) {
                                warnings.push(w);
                            }
                        }
                    }
                }
                depth += 1;
                selector.clear();
            }
            '}' => {
                depth = (depth - 1).max(0);
                selector.clear();
            }
            _ => {
                if depth == 0 {
                    selector.push(ch);
                }
            }
        }
    }
    warnings
}

fn lint_selector(sel: &str) -> Option<String> {
    if sel.is_empty() || sel.starts_with("[data-iii-ui") {
        return None;
    }
    let unscoped_root = sel == "*"
        || sel.starts_with(":root")
        || starts_with_element(sel, "html")
        || starts_with_element(sel, "body");
    let bare_element = sel
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
        && sel
            .split(|c: char| c == ' ' || c == '>' || c == '+' || c == '~' || c == ':' || c == '.' || c == '[' || c == '#')
            .next()
            .is_some_and(|head| {
                !head.is_empty()
                    && head
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            });
    if unscoped_root || bare_element {
        Some(format!(
            "selector '{sel}' is not scoped under [data-iii-ui=\"…\"] and will style the \
             whole console document"
        ))
    } else {
        None
    }
}

fn starts_with_element(sel: &str, element: &str) -> bool {
    sel.strip_prefix(element).is_some_and(|rest| {
        rest.is_empty()
            || rest.starts_with(' ')
            || rest.starts_with('.')
            || rest.starts_with(':')
            || rest.starts_with('[')
            || rest.starts_with('>')
    })
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c2) = chars.next() {
                if c2 == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Seed a registry directly (no bus round-trip) for route tests.
    pub(crate) fn insert_script(
        registry: &Arc<UiRegistry>,
        path: &str,
        content: &str,
        trigger_id: &str,
    ) {
        let mut inner = registry.lock();
        inner
            .by_id
            .insert(trigger_id.to_string(), path.to_string());
        inner.by_path.insert(
            path.to_string(),
            Asset {
                trigger_id: trigger_id.to_string(),
                kind: AssetKind::Script,
                hash: content_hash(content),
                content: content.to_string(),
                content_type: AssetKind::Script.content_type().to_string(),
                warnings: Vec::new(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Scripted bus: canned content per (function_id, path); records pushes
    /// and engine prunes.
    struct MockBus {
        content: StdMutex<HashMap<String, Result<String, String>>>,
        pushes: StdMutex<Vec<(String, Value)>>,
        pruned: StdMutex<Vec<String>>,
    }

    impl MockBus {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                content: StdMutex::new(HashMap::new()),
                pushes: StdMutex::new(Vec::new()),
                pruned: StdMutex::new(Vec::new()),
            })
        }

        fn set_content(&self, path: &str, content: &str) {
            self.content
                .lock()
                .unwrap()
                .insert(path.to_string(), Ok(content.to_string()));
        }

        fn set_error(&self, path: &str, err: &str) {
            self.content
                .lock()
                .unwrap()
                .insert(path.to_string(), Err(err.to_string()));
        }

        fn pushes(&self) -> Vec<(String, Value)> {
            self.pushes.lock().unwrap().clone()
        }

        fn pruned(&self) -> Vec<String> {
            self.pruned.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl UiBus for MockBus {
        async fn fetch_content(
            &self,
            _function_id: &str,
            path: &str,
        ) -> Result<FetchedContent, String> {
            match self.content.lock().unwrap().get(path) {
                Some(Ok(c)) => Ok(FetchedContent {
                    content: c.clone(),
                    content_type: None,
                }),
                Some(Err(e)) => Err(e.clone()),
                None => Err(format!("no canned content for '{path}'")),
            }
        }

        async fn push(&self, function_id: &str, payload: Value) {
            self.pushes
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }

        async fn unregister_engine_trigger(&self, trigger_id: &str) {
            self.pruned.lock().unwrap().push(trigger_id.to_string());
        }
    }

    fn trigger(id: &str, function_id: &str, path: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config: json!({ "path": path }),
            metadata: None,
        }
    }

    fn subscription(id: &str, function_id: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config: json!({}),
            metadata: None,
        }
    }

    fn processor(bus: Arc<MockBus>) -> (Processor, Arc<UiRegistry>) {
        let registry = Arc::new(UiRegistry::default());
        (Processor::new(registry.clone(), bus), registry)
    }

    #[test]
    fn path_validation_matrix() {
        assert!(validate_path("state/page.js").is_ok());
        assert!(validate_path("state/theme.css").is_ok());
        assert!(validate_path("a/b/c-d_e.f.js").is_ok());
        assert!(validate_path("").is_err());
        assert!(validate_path("/state/page.js").is_err());
        assert!(validate_path("state//page.js").is_err());
        assert!(validate_path("state/../page.js").is_err());
        assert!(validate_path("state/./page.js").is_err());
        assert!(validate_path("State/Page.js").is_err());
        assert!(validate_path("state\\page.js").is_err());
        assert!(validate_path("-state/page.js").is_err());
        assert!(validate_path("state/pa ge.js").is_err());
    }

    #[tokio::test]
    async fn register_publishes_and_serves() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "export default () => {}");
        let (p, registry) = processor(bus.clone());

        p.register_asset(AssetKind::Script, trigger("t1", "state::ui-content", "state/page.js"))
            .await
            .unwrap();

        let (content, ctype, hash) = registry.serve("state/page.js").unwrap();
        assert_eq!(content, "export default () => {}");
        assert!(ctype.starts_with("text/javascript"));
        assert_eq!(hash.len(), 16);
        assert_eq!(registry.manifest().len(), 1);
    }

    #[tokio::test]
    async fn extension_must_match_kind() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "x");
        let (p, _) = processor(bus.clone());
        let err = p
            .register_asset(AssetKind::Style, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap_err();
        assert!(err.contains(".css"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn fetch_failure_rejects_registration() {
        let bus = MockBus::new();
        bus.set_error("state/page.js", "worker exploded");
        let (p, registry) = processor(bus.clone());
        let err = p
            .register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap_err();
        assert!(err.contains("worker exploded"));
        assert!(registry.serve("state/page.js").is_none());
    }

    #[tokio::test]
    async fn supersede_prunes_old_id_and_pushes_set() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.register_subscription(subscription("sub1", "iii::console::ui-assets::tab1"))
            .await;
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap();

        bus.set_content("state/page.js", "v2");
        p.register_asset(AssetKind::Script, trigger("t2", "f", "state/page.js"))
            .await
            .unwrap();

        assert_eq!(bus.pruned(), vec!["t1".to_string()]);
        let (content, _, _) = registry.serve("state/page.js").unwrap();
        assert_eq!(content, "v2");

        // sync (subscription) + set (t1) + set (t2)
        let pushes = bus.pushes();
        assert_eq!(pushes.len(), 3);
        assert_eq!(pushes[0].1["event"], "sync");
        assert_eq!(pushes[1].1["event"], "set");
        assert_eq!(pushes[2].1["event"], "set");
        assert_eq!(pushes[2].1["hash"], json!(content_hash("v2")));

        // The superseded trigger's engine echo arrives as an unknown-id
        // no-op — nothing removed, no delete push.
        p.unregister(trigger("t1", "f", "state/page.js")).await;
        assert!(registry.serve("state/page.js").is_some());
        assert_eq!(bus.pushes().len(), 3);
    }

    #[tokio::test]
    async fn replay_same_id_same_hash_is_a_noop() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, _) = processor(bus.clone());
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap();
        p.register_subscription(subscription("sub1", "iii::console::ui-assets::tab1"))
            .await;
        let before = bus.pushes().len();
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap();
        assert_eq!(bus.pushes().len(), before, "replay must publish nothing");
    }

    #[tokio::test]
    async fn id_reuse_under_new_path_is_a_move() {
        let bus = MockBus::new();
        bus.set_content("state/a.js", "a");
        bus.set_content("state/b.js", "b");
        let (p, registry) = processor(bus.clone());
        p.register_subscription(subscription("sub1", "tab1")).await;
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/a.js"))
            .await
            .unwrap();
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/b.js"))
            .await
            .unwrap();

        assert!(registry.serve("state/a.js").is_none());
        assert!(registry.serve("state/b.js").is_some());
        // No engine prune — the engine's row for t1 IS the new binding.
        assert!(bus.pruned().is_empty());
        let events: Vec<String> = bus
            .pushes()
            .iter()
            .map(|(_, p)| p["event"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(events, vec!["sync", "set", "delete", "set"]);
    }

    #[tokio::test]
    async fn unregister_removes_and_pushes_delete() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.register_subscription(subscription("sub1", "tab1")).await;
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap();
        p.unregister(trigger("t1", "f", "state/page.js")).await;
        assert!(registry.serve("state/page.js").is_none());
        let last = bus.pushes().pop().unwrap();
        assert_eq!(last.1["event"], "delete");
        assert_eq!(last.1["path"], "state/page.js");
    }

    #[tokio::test]
    async fn subscription_gets_sync_then_later_commits() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, _) = processor(bus.clone());
        p.register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap();
        p.register_subscription(subscription("sub1", "iii::console::ui-assets::tab1"))
            .await;

        let pushes = bus.pushes();
        assert_eq!(pushes.len(), 1);
        assert_eq!(pushes[0].0, "iii::console::ui-assets::tab1");
        assert_eq!(pushes[0].1["event"], "sync");
        assert_eq!(pushes[0].1["assets"].as_array().unwrap().len(), 1);

        // Subscriber dropped on unregister; later commits push nothing.
        p.unregister(subscription("sub1", "iii::console::ui-assets::tab1"))
            .await;
        bus.set_content("state/page.js", "v2");
        p.register_asset(AssetKind::Script, trigger("t2", "f", "state/page.js"))
            .await
            .unwrap();
        assert_eq!(bus.pushes().len(), 1);
    }

    #[tokio::test]
    async fn oversized_asset_is_rejected() {
        let bus = MockBus::new();
        let big = "x".repeat(MAX_ASSET_BYTES + 1);
        bus.set_content("state/page.js", &big);
        let (p, _) = processor(bus.clone());
        let err = p
            .register_asset(AssetKind::Script, trigger("t1", "f", "state/page.js"))
            .await
            .unwrap_err();
        assert!(err.contains("cap"), "unexpected error: {err}");
    }

    #[test]
    fn style_lint_flags_unscoped_rules_and_font_face() {
        let dirty = "body { margin: 0 } .card { color: red } @font-face { font-family: x }";
        let warnings = lint_style(dirty);
        assert_eq!(warnings.len(), 2, "warnings: {warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("@font-face")));
        assert!(warnings.iter().any(|w| w.contains("'body'")));

        let clean = "[data-iii-ui=\"state\"] .flex { display: flex }\n\
                     [data-iii-ui=\"state\"] { --spacing: 0.25rem }";
        assert!(lint_style(clean).is_empty());

        // Class selectors and comments don't trip it.
        assert!(lint_style("/* body { } */ .a { color: red }").is_empty());
    }

    #[test]
    fn content_hash_is_16_hex_chars_and_stable() {
        let h = content_hash("hello");
        assert_eq!(h.len(), 16);
        assert_eq!(h, content_hash("hello"));
        assert_ne!(h, content_hash("hello2"));
    }
}
