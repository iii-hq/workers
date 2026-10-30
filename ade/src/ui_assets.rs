//! Injectable console UI — the `console::ui::*` functions and asset registry.
//!
//! Implements the injection protocol from
//! `iii/tech-specs/2026-07-17-injectable-ui/injection-protocol.md` with two
//! private functions this worker hosts:
//!
//! - **`console::ui::asset`**: a worker ships one asset by observing it with
//!   args `{ kind: "script" | "style" | "module", path }` and its *content
//!   function* as the target. Once the observer is active, the console calls
//!   the content function through it with `{ path }` and stores the source.
//!   Modules are served like scripts but the SPA loader never imports them
//!   at mount — pages pull them on demand via `host.importModule`. The
//!   function itself is never called.
//! - **`console::ui::on-assets-changed`**: console tabs observe it. Each new
//!   observer receives `{ op: "sync", assets }` through itself once it is
//!   active, and every later change is one call of the function with
//!   `{ op: "set", asset }` or `{ op: "delete", path }`, which iii delivers
//!   to every tab.
//!
//! "Same path ⇒ override" is console-worker policy: a path-keyed
//! last-writer-wins registry. A later observer of a path supersedes the
//! earlier one, which is late-rejected with `UI_ASSET_SUPERSEDED`, so at most
//! one asset observer per path stays active in steady state.
//!
//! The SDK runs observer hooks one at a time per function, and these are two
//! functions, so every added and removed observer of both — and the
//! per-worker toggles — drains through one single-consumer queue in the
//! order the hooks saw them: an asset commit and a tab's `sync` are
//! mutually ordered. The hooks return at once; the queue waits for each
//! observer to be accepted before it calls through it (a call through an
//! observer that is not active yet fails with `OBSERVER_NOT_ACTIVE`).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::{CallAction, CallRequest, IIIClient, ObserveRejection, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

/// The asset function: one observer per asset, never called.
pub const ASSET_FN: &str = "console::ui::asset";
/// The tabs' live feed of asset changes.
pub const ON_ASSETS_CHANGED_FN: &str = "console::ui::on-assets-changed";

/// Late rejection of an asset observer a later observer of its path replaced.
pub const CODE_SUPERSEDED: &str = "UI_ASSET_SUPERSEDED";
/// Late rejection of an asset observer whose content could not be fetched.
pub const CODE_FETCH_FAILED: &str = "UI_ASSET_FETCH_FAILED";
/// Late rejection of an asset observer whose content is over the size cap.
pub const CODE_TOO_LARGE: &str = "UI_ASSET_TOO_LARGE";
/// Rejection of an asset observer whose args name an invalid path.
const CODE_INVALID_ARGS: &str = "INVALID_ARGS";

/// Max bytes per fetched asset. Multi-MiB assets are a bundling smell; the
/// cap keeps a misbehaving worker from ballooning console memory.
const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;

/// Content-fetch budget per asset: 2 attempts × 3s + 250ms backoff ≈ 6.5s
/// worst case. The fetch runs inside the serialized queue, so the budget
/// bounds how long one slow worker holds up every later asset.
const FETCH_ATTEMPTS: u32 = 2;
const FETCH_TIMEOUT_MS: u64 = 3_000;
const FETCH_BACKOFF_MS: u64 = 250;

/// Args of a `console::ui::asset` observer (`additionalProperties` refused).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetArgs {
    /// `script` (imported at mount), `style` (mounted as a `<link>`) or
    /// `module` (served, imported on demand with `host.importModule`).
    pub kind: AssetKind,
    /// Asset identity. Convention: `<worker>/<name>.<ext>`. A later observer
    /// of the same path supersedes this one.
    pub path: String,
}

/// Asset kind, as it travels in the observer's args, the events and the
/// manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Script,
    Style,
    Module,
}

impl AssetKind {
    pub fn content_type(self) -> &'static str {
        match self {
            AssetKind::Script | AssetKind::Module => "text/javascript; charset=utf-8",
            AssetKind::Style => "text/css; charset=utf-8",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            AssetKind::Script | AssetKind::Module => ".js",
            AssetKind::Style => ".css",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            AssetKind::Script => "script",
            AssetKind::Style => "style",
            AssetKind::Module => "module",
        }
    }
}

/// One served asset, as the events name it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AssetRef {
    pub path: String,
    pub kind: AssetKind,
    /// First 16 hex chars of sha256(content).
    pub hash: String,
}

/// The event `console::ui::on-assets-changed` is called with, and what its
/// observers receive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum UiAssetsEvent {
    /// The whole served set, sent to one new observer through itself.
    Sync { assets: Vec<AssetRef> },
    /// An asset was added, or its content changed.
    Set { asset: AssetRef },
    /// An asset is no longer served.
    Delete { path: String },
}

#[derive(Debug, Clone)]
pub struct Asset {
    /// The `console::ui::asset` observer that ships it.
    pub observer_id: String,
    pub kind: AssetKind,
    /// First 16 hex chars of sha256(content) — the version that travels in
    /// events, the manifest, and `?v=` cache busting.
    pub hash: String,
    pub content: String,
    pub content_type: String,
    /// Style-lint findings (warn-only). Empty for scripts, modules, and clean
    /// styles.
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

/// Per-worker summary in the manifest: every worker with at least one
/// *registered* asset — including workers currently toggled off, whose
/// assets are held but not served/announced.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ManifestWorker {
    /// Worker name — the first segment of its asset paths.
    pub worker: String,
    /// `false` while the worker is in the console configuration's
    /// `injectableUi.disabledWorkers` list.
    pub enabled: bool,
    /// Number of registered assets (served only while enabled).
    pub assets: usize,
}

/// The scope a path belongs to — its first segment, by the authoring
/// convention the registering worker's name.
fn worker_of(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

/// The console's own injected UI ships the form that flips these toggles —
/// letting it disable itself would lock the operator out of the UI that
/// re-enables workers. Guarded here (the enforcement point), not just in
/// the form.
const UNDISABLEABLE_WORKER: &str = "console";

#[derive(Default)]
struct RegistryInner {
    /// path → asset (content pinned to the advertised hash).
    by_path: HashMap<String, Asset>,
    /// asset observer id → path. A removed observer carries only its id
    /// here, hence the second key.
    by_id: HashMap<String, String>,
    /// Workers whose assets are held but not served, announced, or listed
    /// (the per-worker injectable-UI toggle, from the console configuration
    /// entry). Observers are still accepted while disabled so a toggle back
    /// on restores the UI without worker restarts.
    disabled_workers: HashSet<String>,
}

impl RegistryInner {
    fn is_enabled(&self, path: &str) -> bool {
        !self.disabled_workers.contains(worker_of(path))
    }

    /// The served set, as a `sync` names it.
    fn served(&self) -> Vec<AssetRef> {
        let mut assets: Vec<AssetRef> = self
            .by_path
            .iter()
            .filter(|(path, _)| self.is_enabled(path))
            .map(|(path, a)| AssetRef {
                path: path.clone(),
                kind: a.kind,
                hash: a.hash.clone(),
            })
            .collect();
        assets.sort_by(|a, b| a.path.cmp(&b.path));
        assets
    }
}

/// In-memory asset registry. Rebuilt from the observers the engine announces
/// again on console restart; deliberately not persisted.
#[derive(Default)]
pub struct UiRegistry {
    inner: Mutex<RegistryInner>,
}

impl UiRegistry {
    /// Bytes + headers for `GET /ui/<path>`. Disabled workers' assets are
    /// held but not served.
    pub fn serve(&self, path: &str) -> Option<(String, String, String)> {
        let inner = self.lock();
        if !inner.is_enabled(path) {
            return None;
        }
        let asset = inner.by_path.get(path)?;
        Some((
            asset.content.clone(),
            asset.content_type.clone(),
            asset.hash.clone(),
        ))
    }

    /// The loadable set — disabled workers' assets are excluded, exactly
    /// like the `sync` sent to observing tabs.
    pub fn manifest(&self) -> Vec<ManifestAsset> {
        let inner = self.lock();
        let mut assets: Vec<ManifestAsset> = inner
            .by_path
            .iter()
            .filter(|(path, _)| inner.is_enabled(path))
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

    /// Per-worker view for the manifest and the injectable-UI toggle form:
    /// every worker with a registered asset (enabled or not), plus disabled
    /// workers with nothing registered right now (their toggle must stay
    /// visible so they can be re-enabled).
    pub fn workers(&self) -> Vec<ManifestWorker> {
        let inner = self.lock();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for path in inner.by_path.keys() {
            *counts.entry(worker_of(path)).or_default() += 1;
        }
        let mut workers: Vec<ManifestWorker> = counts
            .iter()
            .map(|(worker, count)| ManifestWorker {
                worker: (*worker).to_string(),
                enabled: !inner.disabled_workers.contains(*worker),
                assets: *count,
            })
            .collect();
        for disabled in &inner.disabled_workers {
            if !counts.contains_key(disabled.as_str()) {
                workers.push(ManifestWorker {
                    worker: disabled.clone(),
                    enabled: false,
                    assets: 0,
                });
            }
        }
        workers.sort_by(|a, b| a.worker.cmp(&b.worker));
        workers
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// What the queue consumer needs from the bus, abstracted so the override
/// algorithm is unit-testable without an engine (the state worker's
/// `Invoker` precedent). Observers are named by id: the bus holds them.
#[async_trait]
pub trait UiBus: Send + Sync + 'static {
    /// Resolves `true` once the observer is accepted (active), `false` when
    /// it was rejected or removed first.
    async fn accepted(&self, observer_id: &str) -> bool;
    /// Call an asset observer's target (its worker's content function)
    /// through the observer: `{path}` → `{content, content_type?}`.
    async fn fetch_content(&self, observer_id: &str, path: &str) -> Result<FetchedContent, String>;
    /// Send one `console::ui::on-assets-changed` observer an event through
    /// itself.
    async fn send(&self, observer_id: &str, event: UiAssetsEvent);
    /// Call `console::ui::on-assets-changed` with an event, which iii
    /// delivers to every tab observing it.
    async fn publish(&self, event: UiAssetsEvent);
    /// Late-reject an asset observer: it stops being active, and its
    /// removed hook runs.
    fn reject(&self, observer_id: &str, code: &str, message: &str);
}

#[derive(Debug, Clone)]
pub struct FetchedContent {
    pub content: String,
    pub content_type: Option<String>,
}

/// The observers of both functions the hooks handed over, by id.
#[derive(Default)]
struct Observers(Mutex<HashMap<String, Observer>>);

impl Observers {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Observer>> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn get(&self, id: &str) -> Option<Observer> {
        self.lock().get(id).cloned()
    }
}

/// Whether a call through an observer was refused because the observer is
/// not active. The engine announces the observers of a worker that
/// disconnected (`suspended` for `observers.observer_grace_ms`) to the
/// function's host too, and refuses calls through them until that worker
/// reconnects: a restarted console meets its previous process's asset
/// observers this way. The fetch gives up on the first such refusal, since
/// every later asset waits in the queue behind it.
fn is_inactive_observer(error: &Error) -> bool {
    matches!(error, Error::Remote { code, .. } if code == observe_codes::OBSERVER_NOT_ACTIVE)
}

struct SdkBus {
    iii: Arc<IIIClient>,
    observers: Arc<Observers>,
}

#[async_trait]
impl UiBus for SdkBus {
    async fn accepted(&self, observer_id: &str) -> bool {
        match self.observers.get(observer_id) {
            Some(obs) => obs.accepted().await.is_ok(),
            None => false,
        }
    }

    async fn fetch_content(&self, observer_id: &str, path: &str) -> Result<FetchedContent, String> {
        let obs = self
            .observers
            .get(observer_id)
            .ok_or_else(|| format!("observer {observer_id} was removed"))?;
        let mut last_err = String::new();
        let mut attempts = 0;
        for attempt in 1..=FETCH_ATTEMPTS {
            attempts = attempt;
            let request = CallRequest::observer(&obs, json!({ "path": path }))
                .timeout(Duration::from_millis(FETCH_TIMEOUT_MS));
            match self.iii.call(request).await {
                Ok(resp) => {
                    let content = resp
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| {
                            format!(
                                "content function '{}' returned no string `content` field \
                                 for path '{path}'",
                                obs.target()
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
                    if is_inactive_observer(&e) {
                        break;
                    }
                    if attempt < FETCH_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(FETCH_BACKOFF_MS)).await;
                    }
                }
            }
        }
        Err(format!(
            "content fetch via '{}' failed after {attempts} attempt(s): {last_err}",
            obs.target()
        ))
    }

    async fn send(&self, observer_id: &str, event: UiAssetsEvent) {
        let Some(obs) = self.observers.get(observer_id) else {
            return;
        };
        let payload = serde_json::to_value(event).expect("UiAssetsEvent serializes");
        let request = CallRequest::observer(&obs, payload).action(CallAction::Void);
        if let Err(e) = self.iii.call(request).await {
            // A closed tab's observer is removed by the engine moments later.
            tracing::debug!(observer_id, error = %e, "ui-assets sync failed");
        }
    }

    async fn publish(&self, event: UiAssetsEvent) {
        if !self.iii.is_observed(ON_ASSETS_CHANGED_FN) {
            return;
        }
        let payload = serde_json::to_value(event).expect("UiAssetsEvent serializes");
        let request = CallRequest {
            function_id: ON_ASSETS_CHANGED_FN.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(e) = self.iii.call(request).await {
            tracing::debug!(error = %e, "ui-assets publish failed");
        }
    }

    fn reject(&self, observer_id: &str, code: &str, message: &str) {
        if let Some(obs) = self.observers.get(observer_id) {
            obs.reject(code, message);
        }
    }
}

enum QueueEvent {
    /// A `console::ui::asset` observer was added: once accepted, fetch its
    /// content and publish it.
    AssetAdded {
        observer_id: String,
        args: AssetArgs,
    },
    /// A `console::ui::asset` observer was removed.
    AssetRemoved {
        observer_id: String,
        ack: oneshot::Sender<()>,
    },
    /// A `console::ui::on-assets-changed` observer was added: once accepted,
    /// send it the served set.
    TabAdded { observer_id: String },
    /// Replace the per-worker disable set (from the console configuration
    /// entry). Queued so the resulting delete/set events are mutually
    /// ordered with asset commits and tab syncs.
    SetDisabledWorkers {
        workers: HashSet<String>,
        ack: oneshot::Sender<()>,
    },
}

/// Runtime control over the registry, safe to hold outside the queue —
/// currently just the per-worker injectable-UI toggle.
#[derive(Clone)]
pub struct UiControl {
    tx: mpsc::UnboundedSender<QueueEvent>,
}

impl UiControl {
    /// Replace the disabled-worker set. Resolves after the change and its
    /// catch-up events (deletes for newly disabled workers' assets, sets
    /// for newly enabled ones) are applied.
    pub async fn set_disabled_workers(&self, workers: HashSet<String>) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self
            .tx
            .send(QueueEvent::SetDisabledWorkers {
                workers,
                ack: ack_tx,
            })
            .is_err()
        {
            tracing::warn!("ui asset queue is gone; disabled-workers change dropped");
            return;
        }
        let _ = ack_rx.await;
    }
}

/// Validate a `console::ui::asset` observer's args: the path rules and an
/// extension that fits the kind.
fn validate_args(args: &AssetArgs) -> Result<(), String> {
    validate_path(&args.path)?;
    if !args.path.ends_with(args.kind.extension()) {
        return Err(format!(
            "path '{}' must end in '{}' for a {} asset",
            args.path,
            args.kind.extension(),
            args.kind.as_str(),
        ));
    }
    Ok(())
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
                QueueEvent::AssetAdded { observer_id, args } => {
                    if self.bus.accepted(&observer_id).await {
                        self.commit_asset(&observer_id, args).await;
                    }
                }
                QueueEvent::AssetRemoved { observer_id, ack } => {
                    self.remove_asset(&observer_id).await;
                    let _ = ack.send(());
                }
                QueueEvent::TabAdded { observer_id } => {
                    if self.bus.accepted(&observer_id).await {
                        self.sync_tab(&observer_id).await;
                    }
                }
                QueueEvent::SetDisabledWorkers { workers, ack } => {
                    self.apply_disabled_workers(workers).await;
                    let _ = ack.send(());
                }
            }
        }
    }

    /// Apply the per-worker toggle set: newly disabled workers' assets get a
    /// `delete` (they vanish live), newly enabled ones a `set` (tabs load
    /// them without a worker restart). Serving and the manifest follow the
    /// same set via [`RegistryInner::is_enabled`].
    async fn apply_disabled_workers(&self, mut next: HashSet<String>) {
        if next.remove(UNDISABLEABLE_WORKER) {
            tracing::warn!(
                "'{UNDISABLEABLE_WORKER}' cannot have its injected UI disabled \
                 (it ships the toggle form) — ignoring that entry"
            );
        }
        let mut deletes: Vec<String> = Vec::new();
        let mut sets: Vec<AssetRef> = Vec::new();
        {
            let mut inner = self.registry.lock();
            let prev = std::mem::replace(&mut inner.disabled_workers, next);
            for (path, asset) in &inner.by_path {
                let worker = worker_of(path);
                let was_disabled = prev.contains(worker);
                let is_disabled = inner.disabled_workers.contains(worker);
                if is_disabled && !was_disabled {
                    deletes.push(path.clone());
                } else if was_disabled && !is_disabled {
                    sets.push(AssetRef {
                        path: path.clone(),
                        kind: asset.kind,
                        hash: asset.hash.clone(),
                    });
                }
            }
            tracing::info!(
                disabled = ?inner.disabled_workers,
                "injectable-ui per-worker toggles applied"
            );
        }
        deletes.sort();
        sets.sort_by(|a, b| a.path.cmp(&b.path));
        for path in deletes {
            self.bus.publish(UiAssetsEvent::Delete { path }).await;
        }
        for asset in sets {
            self.bus.publish(UiAssetsEvent::Set { asset }).await;
        }
    }

    /// Send a newly active tab the served set. Whatever committed before it
    /// is in the sync; whatever commits after reaches the tab as a call of
    /// the function (same queue), so there is no seed/event race to speak
    /// of.
    async fn sync_tab(&self, observer_id: &str) {
        let assets = self.registry.lock().served();
        tracing::info!(
            observer_id,
            assets = assets.len(),
            "{ON_ASSETS_CHANGED_FN} observer synced"
        );
        self.bus
            .send(observer_id, UiAssetsEvent::Sync { assets })
            .await;
    }

    /// The override algorithm — idempotent, as an observer accepted again
    /// with unchanged content publishes nothing.
    async fn commit_asset(&self, observer_id: &str, args: AssetArgs) {
        let AssetArgs { kind, path } = args;
        let fetched = match self.bus.fetch_content(observer_id, &path).await {
            Ok(fetched) => fetched,
            Err(error) => {
                tracing::warn!(path = %path, observer_id, %error, "ui asset fetch failed");
                self.bus.reject(observer_id, CODE_FETCH_FAILED, &error);
                return;
            }
        };
        if fetched.content.len() > MAX_ASSET_BYTES {
            let message = format!(
                "asset '{path}' is {} bytes — over the {MAX_ASSET_BYTES}-byte cap; \
                 check the worker's bundling",
                fetched.content.len()
            );
            tracing::warn!(path = %path, observer_id, "{message}");
            self.bus.reject(observer_id, CODE_TOO_LARGE, &message);
            return;
        }
        let hash = content_hash(&fetched.content);
        let warnings = match kind {
            AssetKind::Style => lint_style(&fetched.content),
            AssetKind::Script | AssetKind::Module => Vec::new(),
        };
        for w in &warnings {
            tracing::warn!(path = %path, "style lint: {w}");
        }
        let asset = Asset {
            observer_id: observer_id.to_string(),
            kind,
            hash: hash.clone(),
            content: fetched.content,
            content_type: fetched
                .content_type
                .unwrap_or_else(|| kind.content_type().to_string()),
            warnings,
        };

        // Commit under one lock; bus calls (rejection + publish) happen
        // after, still inside the serialized queue section.
        let mut superseded: Option<String> = None;
        let enabled: bool;
        {
            let mut inner = self.registry.lock();
            enabled = inner.is_enabled(&path);

            if let Some(existing) = inner.by_path.get(&path) {
                if existing.observer_id == observer_id && existing.hash == hash {
                    // Accepted again, nothing changed: publish nothing.
                    return;
                }
                if existing.observer_id != observer_id {
                    // Supersede: last writer wins, including across workers.
                    tracing::warn!(
                        path = %path,
                        old_observer = %existing.observer_id,
                        new_observer = %observer_id,
                        "ui asset path observed again; superseding the previous observer"
                    );
                    let old_id = existing.observer_id.clone();
                    inner.by_id.remove(&old_id);
                    superseded = Some(old_id);
                }
            }

            inner.by_id.insert(observer_id.to_string(), path.clone());
            inner.by_path.insert(path.clone(), asset);
        }

        if let Some(old_id) = superseded {
            self.bus.reject(
                &old_id,
                CODE_SUPERSEDED,
                &format!("a later observer of '{path}' replaced this asset"),
            );
        }
        if !enabled {
            // Held, not published: no event, no serve, no manifest row —
            // toggling the worker back on publishes `set` for everything held.
            tracing::info!(
                path = %path, hash = %hash, kind = kind.as_str(),
                "ui asset stored (worker's injectable ui is disabled)"
            );
            return;
        }
        tracing::info!(path = %path, hash = %hash, kind = kind.as_str(), "ui asset published");
        self.bus
            .publish(UiAssetsEvent::Set {
                asset: AssetRef { path, kind, hash },
            })
            .await;
    }

    /// Unknown ids are no-ops — that is exactly what a superseded observer's
    /// own removal looks like.
    async fn remove_asset(&self, observer_id: &str) {
        let mut removed: Option<String> = None;
        {
            let mut inner = self.registry.lock();
            if let Some(path) = inner.by_id.remove(observer_id) {
                let matches = inner
                    .by_path
                    .get(&path)
                    .is_some_and(|a| a.observer_id == observer_id);
                if matches && inner.is_enabled(&path) {
                    inner.by_path.remove(&path);
                    removed = Some(path);
                } else if matches {
                    // Disabled worker: tabs never saw the asset, so no
                    // delete — just drop the held bytes.
                    inner.by_path.remove(&path);
                    tracing::info!(path = %path, "held ui asset removed (worker disabled)");
                }
            }
        }
        if let Some(path) = removed {
            tracing::info!(path = %path, "ui asset removed");
            self.bus.publish(UiAssetsEvent::Delete { path }).await;
        }
    }
}

/// Register `console::ui::asset` and `console::ui::on-assets-changed` with
/// their observer hooks, and spawn the queue consumer. Returns the registry
/// the HTTP routes and `console::ui-manifest` read from, plus the control
/// handle the configuration module feeds the per-worker toggles through.
pub fn start(iii: &Arc<IIIClient>) -> (Arc<UiRegistry>, UiControl) {
    let registry = Arc::new(UiRegistry::default());
    let observers = Arc::new(Observers::default());
    let bus = Arc::new(SdkBus {
        iii: iii.clone(),
        observers: observers.clone(),
    });
    let (tx, rx) = mpsc::unbounded_channel();
    let control = UiControl { tx: tx.clone() };
    tokio::spawn(Processor::new(registry.clone(), bus).run(rx));

    // Hooks by id before registering, so they see every observer.
    {
        let observers = observers.clone();
        let tx = tx.clone();
        iii.on_observer_added_for(ASSET_FN, move |obs: Observer| {
            let observers = observers.clone();
            let tx = tx.clone();
            async move {
                let args: AssetArgs = obs.args()?;
                validate_args(&args).map_err(|e| ObserveRejection::new(CODE_INVALID_ARGS, e))?;
                let observer_id = obs.id().to_string();
                observers.lock().insert(observer_id.clone(), obs);
                tx.send(QueueEvent::AssetAdded { observer_id, args })
                    .map_err(|_| ObserveRejection::from("ui asset queue is gone".to_string()))
            }
        });
    }
    {
        let observers = observers.clone();
        let tx = tx.clone();
        iii.on_observer_removed_for(ASSET_FN, move |obs: Observer| {
            let observers = observers.clone();
            let tx = tx.clone();
            async move {
                let (ack, done) = oneshot::channel();
                let observer_id = obs.id().to_string();
                if tx
                    .send(QueueEvent::AssetRemoved {
                        observer_id: observer_id.clone(),
                        ack,
                    })
                    .is_ok()
                {
                    let _ = done.await;
                }
                observers.lock().remove(&observer_id);
            }
        });
    }
    {
        let observers = observers.clone();
        let tx = tx.clone();
        iii.on_observer_added_for(ON_ASSETS_CHANGED_FN, move |obs: Observer| {
            let observers = observers.clone();
            let tx = tx.clone();
            async move {
                let observer_id = obs.id().to_string();
                observers.lock().insert(observer_id.clone(), obs);
                tx.send(QueueEvent::TabAdded { observer_id })
                    .map_err(|_| ObserveRejection::from("ui asset queue is gone".to_string()))
            }
        });
    }
    {
        let observers = observers.clone();
        iii.on_observer_removed_for(ON_ASSETS_CHANGED_FN, move |obs: Observer| {
            let observers = observers.clone();
            async move {
                observers.lock().remove(obs.id());
            }
        });
    }

    iii.register_function(
        ASSET_FN,
        RegisterFunction::new(|input: Value| Ok::<_, Error>(input))
            .description(
                "A worker's injected console UI asset. Observe it once per asset with args \
                 { kind: 'script' | 'style' | 'module', path } and the worker's content \
                 function as the target: once the observer is active, the console calls the \
                 content function through it with { path } and serves what it returns as \
                 { content, content_type? }. A later observer of the same path supersedes \
                 the earlier one (UI_ASSET_SUPERSEDED); a failed fetch rejects the observer \
                 (UI_ASSET_FETCH_FAILED). Never called.",
            )
            .args_format::<AssetArgs>()
            .private(),
    );
    iii.register_function(
        ON_ASSETS_CHANGED_FN,
        RegisterFunction::new(|event: UiAssetsEvent| Ok::<_, Error>(event))
            .description(
                "Called on every change to the console's injected UI assets: \
                 { op: 'set', asset: { path, kind, hash } } or { op: 'delete', path }. \
                 Each new observer first receives { op: 'sync', assets } through itself, \
                 the whole served set.",
            )
            .private(),
    );
    tracing::info!(
        functions = ?[ASSET_FN, ON_ASSETS_CHANGED_FN],
        "registered injectable-ui functions"
    );
    (registry, control)
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
    let bare_element = sel.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && sel
            .split(|c: char| {
                c == ' '
                    || c == '>'
                    || c == '+'
                    || c == '~'
                    || c == ':'
                    || c == '.'
                    || c == '['
                    || c == '#'
            })
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
        observer_id: &str,
    ) {
        let mut inner = registry.lock();
        inner
            .by_id
            .insert(observer_id.to_string(), path.to_string());
        inner.by_path.insert(
            path.to_string(),
            Asset {
                observer_id: observer_id.to_string(),
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

    /// Scripted bus: canned content per path; records what it sends,
    /// publishes and rejects. Every observer is accepted unless refused.
    struct MockBus {
        content: StdMutex<HashMap<String, Result<String, String>>>,
        refused: StdMutex<HashSet<String>>,
        sent: StdMutex<Vec<(String, UiAssetsEvent)>>,
        published: StdMutex<Vec<UiAssetsEvent>>,
        rejected: StdMutex<Vec<(String, String)>>,
    }

    impl MockBus {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                content: StdMutex::new(HashMap::new()),
                refused: StdMutex::new(HashSet::new()),
                sent: StdMutex::new(Vec::new()),
                published: StdMutex::new(Vec::new()),
                rejected: StdMutex::new(Vec::new()),
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

        fn refuse(&self, observer_id: &str) {
            self.refused.lock().unwrap().insert(observer_id.to_string());
        }

        fn sent(&self) -> Vec<(String, UiAssetsEvent)> {
            self.sent.lock().unwrap().clone()
        }

        fn published(&self) -> Vec<UiAssetsEvent> {
            self.published.lock().unwrap().clone()
        }

        fn rejected(&self) -> Vec<(String, String)> {
            self.rejected.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl UiBus for MockBus {
        async fn accepted(&self, observer_id: &str) -> bool {
            !self.refused.lock().unwrap().contains(observer_id)
        }

        async fn fetch_content(
            &self,
            _observer_id: &str,
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

        async fn send(&self, observer_id: &str, event: UiAssetsEvent) {
            self.sent
                .lock()
                .unwrap()
                .push((observer_id.to_string(), event));
        }

        async fn publish(&self, event: UiAssetsEvent) {
            self.published.lock().unwrap().push(event);
        }

        fn reject(&self, observer_id: &str, code: &str, _message: &str) {
            self.rejected
                .lock()
                .unwrap()
                .push((observer_id.to_string(), code.to_string()));
        }
    }

    fn script(path: &str) -> AssetArgs {
        AssetArgs {
            kind: AssetKind::Script,
            path: path.to_string(),
        }
    }

    fn processor(bus: Arc<MockBus>) -> (Processor, Arc<UiRegistry>) {
        let registry = Arc::new(UiRegistry::default());
        (Processor::new(registry.clone(), bus), registry)
    }

    fn set_of(path: &str, kind: AssetKind, content: &str) -> UiAssetsEvent {
        UiAssetsEvent::Set {
            asset: AssetRef {
                path: path.to_string(),
                kind,
                hash: content_hash(content),
            },
        }
    }

    fn delete_of(path: &str) -> UiAssetsEvent {
        UiAssetsEvent::Delete {
            path: path.to_string(),
        }
    }

    /// Drive the queue consumer the way the hooks do.
    async fn run_events(p: Processor, events: Vec<QueueEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        for event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        p.run(rx).await;
    }

    #[test]
    fn only_an_inactive_observer_refusal_ends_the_fetch_early() {
        let remote = |code: &str| Error::Remote {
            code: code.to_string(),
            message: "refused".to_string(),
            stacktrace: None,
        };
        assert!(is_inactive_observer(&remote(
            observe_codes::OBSERVER_NOT_ACTIVE
        )));
        assert!(!is_inactive_observer(&remote(observe_codes::OVERFLOW)));
        assert!(!is_inactive_observer(&remote("function_not_found")));
        assert!(!is_inactive_observer(&Error::Timeout));
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
        assert!(validate_path(&format!("state/{}.js", "a".repeat(512))).is_err());
    }

    #[test]
    fn args_extension_must_match_kind() {
        assert!(validate_args(&script("state/page.js")).is_ok());
        let err = validate_args(&AssetArgs {
            kind: AssetKind::Style,
            path: "state/page.js".to_string(),
        })
        .unwrap_err();
        assert!(err.contains(".css"), "unexpected error: {err}");
        let err = validate_args(&AssetArgs {
            kind: AssetKind::Module,
            path: "ide/xterm.css".to_string(),
        })
        .unwrap_err();
        assert!(
            err.contains(".js") && err.contains("module"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn args_are_kind_and_path_and_nothing_else() {
        let args: AssetArgs =
            serde_json::from_value(json!({ "kind": "module", "path": "ide/xterm.js" })).unwrap();
        assert_eq!(args.kind, AssetKind::Module);
        assert!(serde_json::from_value::<AssetArgs>(
            json!({ "kind": "script", "path": "a/b.js", "extra": 1 })
        )
        .is_err());
        assert!(serde_json::from_value::<AssetArgs>(json!({ "path": "a/b.js" })).is_err());
    }

    #[test]
    fn events_carry_the_op_tag() {
        assert_eq!(
            serde_json::to_value(UiAssetsEvent::Sync { assets: Vec::new() }).unwrap(),
            json!({ "op": "sync", "assets": [] })
        );
        assert_eq!(
            serde_json::to_value(set_of("state/page.js", AssetKind::Script, "x")).unwrap(),
            json!({
                "op": "set",
                "asset": { "path": "state/page.js", "kind": "script", "hash": content_hash("x") }
            })
        );
        assert_eq!(
            serde_json::to_value(delete_of("state/page.js")).unwrap(),
            json!({ "op": "delete", "path": "state/page.js" })
        );
    }

    #[tokio::test]
    async fn an_accepted_asset_is_fetched_published_and_served() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "export default () => {}");
        let (p, registry) = processor(bus.clone());

        p.commit_asset("o1", script("state/page.js")).await;

        let (content, ctype, hash) = registry.serve("state/page.js").unwrap();
        assert_eq!(content, "export default () => {}");
        assert!(ctype.starts_with("text/javascript"));
        assert_eq!(hash.len(), 16);
        assert_eq!(registry.manifest().len(), 1);
        assert_eq!(
            bus.published(),
            vec![set_of(
                "state/page.js",
                AssetKind::Script,
                "export default () => {}"
            )]
        );
    }

    #[tokio::test]
    async fn module_is_listed_synced_and_served_as_javascript() {
        let bus = MockBus::new();
        bus.set_content("ide/xterm.js", "export const x = 1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset(
            "o1",
            AssetArgs {
                kind: AssetKind::Module,
                path: "ide/xterm.js".to_string(),
            },
        )
        .await;
        p.sync_tab("tab1").await;

        let manifest = serde_json::to_value(registry.manifest()).unwrap();
        assert_eq!(manifest[0]["kind"], "module");
        let UiAssetsEvent::Sync { assets } = &bus.sent()[0].1 else {
            panic!("expected a sync");
        };
        assert_eq!(assets[0].kind, AssetKind::Module);
        let (_, ctype, _) = registry.serve("ide/xterm.js").unwrap();
        assert!(ctype.starts_with("text/javascript"));
    }

    #[tokio::test]
    async fn a_failed_fetch_rejects_the_observer() {
        let bus = MockBus::new();
        bus.set_error("state/page.js", "worker exploded");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        assert_eq!(
            bus.rejected(),
            vec![("o1".to_string(), CODE_FETCH_FAILED.to_string())]
        );
        assert!(registry.serve("state/page.js").is_none());
        assert!(bus.published().is_empty());
    }

    #[tokio::test]
    async fn an_oversized_asset_rejects_the_observer() {
        let bus = MockBus::new();
        let big = "x".repeat(MAX_ASSET_BYTES + 1);
        bus.set_content("state/page.js", &big);
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        assert_eq!(
            bus.rejected(),
            vec![("o1".to_string(), CODE_TOO_LARGE.to_string())]
        );
        assert!(registry.serve("state/page.js").is_none());
    }

    #[tokio::test]
    async fn a_later_observer_supersedes_and_the_old_ones_removal_is_a_noop() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;

        bus.set_content("state/page.js", "v2");
        p.commit_asset("o2", script("state/page.js")).await;

        assert_eq!(
            bus.rejected(),
            vec![("o1".to_string(), CODE_SUPERSEDED.to_string())]
        );
        let (content, _, _) = registry.serve("state/page.js").unwrap();
        assert_eq!(content, "v2");
        assert_eq!(
            bus.published(),
            vec![
                set_of("state/page.js", AssetKind::Script, "v1"),
                set_of("state/page.js", AssetKind::Script, "v2"),
            ]
        );

        // The superseded observer's removal arrives as an unknown id —
        // nothing removed, no delete.
        p.remove_asset("o1").await;
        assert!(registry.serve("state/page.js").is_some());
        assert_eq!(bus.published().len(), 2);
    }

    #[tokio::test]
    async fn the_same_observer_with_the_same_content_publishes_nothing() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, _) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        let before = bus.published().len();
        p.commit_asset("o1", script("state/page.js")).await;
        assert_eq!(bus.published().len(), before, "nothing changed");
        assert!(bus.rejected().is_empty());
    }

    #[tokio::test]
    async fn removal_drops_the_asset_and_publishes_delete() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        p.remove_asset("o1").await;
        assert!(registry.serve("state/page.js").is_none());
        assert_eq!(bus.published().pop().unwrap(), delete_of("state/page.js"));
    }

    #[tokio::test]
    async fn the_queue_commits_in_arrival_order_and_skips_refused_observers() {
        let bus = MockBus::new();
        bus.set_content("ide/xterm.js", "m");
        bus.set_content("ide/page.js", "s");
        bus.set_content("ide/late.js", "l");
        bus.refuse("o3");
        let (p, registry) = processor(bus.clone());
        let (ack, _done) = oneshot::channel();
        run_events(
            p,
            vec![
                QueueEvent::AssetAdded {
                    observer_id: "o1".to_string(),
                    args: AssetArgs {
                        kind: AssetKind::Module,
                        path: "ide/xterm.js".to_string(),
                    },
                },
                QueueEvent::AssetAdded {
                    observer_id: "o2".to_string(),
                    args: script("ide/page.js"),
                },
                QueueEvent::TabAdded {
                    observer_id: "tab1".to_string(),
                },
                QueueEvent::AssetAdded {
                    observer_id: "o3".to_string(),
                    args: script("ide/late.js"),
                },
                QueueEvent::AssetRemoved {
                    observer_id: "o2".to_string(),
                    ack,
                },
            ],
        )
        .await;

        assert_eq!(
            bus.published(),
            vec![
                set_of("ide/xterm.js", AssetKind::Module, "m"),
                set_of("ide/page.js", AssetKind::Script, "s"),
                delete_of("ide/page.js"),
            ]
        );
        let sent = bus.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "tab1");
        let UiAssetsEvent::Sync { assets } = &sent[0].1 else {
            panic!("expected a sync");
        };
        let paths: Vec<&str> = assets.iter().map(|a| a.path.as_str()).collect();
        assert_eq!(paths, vec!["ide/page.js", "ide/xterm.js"]);
        assert!(registry.serve("ide/late.js").is_none(), "o3 was refused");
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

    fn disabled(workers: &[&str]) -> HashSet<String> {
        workers.iter().map(|w| w.to_string()).collect()
    }

    #[tokio::test]
    async fn disabling_a_worker_hides_and_deletes_its_assets() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        bus.set_content("other/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        p.commit_asset("o2", script("other/page.js")).await;

        p.apply_disabled_workers(disabled(&["state"])).await;

        // Held, not served or listed; the other worker is untouched.
        assert!(registry.serve("state/page.js").is_none());
        assert!(registry.serve("other/page.js").is_some());
        assert_eq!(registry.manifest().len(), 1);
        let workers = registry.workers();
        assert_eq!(workers.len(), 2);
        let state = workers.iter().find(|w| w.worker == "state").unwrap();
        assert!(!state.enabled);
        assert_eq!(state.assets, 1);

        // Tabs got a delete for exactly the disabled worker's asset.
        assert_eq!(bus.published().pop().unwrap(), delete_of("state/page.js"));
    }

    #[tokio::test]
    async fn reenabling_publishes_set_and_serves_again() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        p.apply_disabled_workers(disabled(&["state"])).await;
        p.apply_disabled_workers(disabled(&[])).await;

        assert!(registry.serve("state/page.js").is_some());
        assert_eq!(
            bus.published().pop().unwrap(),
            set_of("state/page.js", AssetKind::Script, "v1")
        );
    }

    #[tokio::test]
    async fn an_asset_observed_while_disabled_is_held_then_published_on_enable() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.apply_disabled_workers(disabled(&["state"])).await;
        p.sync_tab("tab1").await;

        // Accepted (the worker's observer must not fail) but invisible.
        p.commit_asset("o1", script("state/page.js")).await;
        assert!(bus.rejected().is_empty());
        assert!(registry.serve("state/page.js").is_none());
        assert!(registry.manifest().is_empty());
        assert!(bus.published().is_empty());
        assert_eq!(
            bus.sent(),
            vec![(
                "tab1".to_string(),
                UiAssetsEvent::Sync { assets: Vec::new() }
            )]
        );

        p.apply_disabled_workers(disabled(&[])).await;
        assert!(registry.serve("state/page.js").is_some());
        assert_eq!(
            bus.published().pop().unwrap(),
            set_of("state/page.js", AssetKind::Script, "v1")
        );
    }

    #[tokio::test]
    async fn removal_while_disabled_drops_silently() {
        let bus = MockBus::new();
        bus.set_content("state/page.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("state/page.js")).await;
        p.apply_disabled_workers(disabled(&["state"])).await;
        let before = bus.published().len();

        p.remove_asset("o1").await;

        // No delete (tabs already dropped it at disable time), and nothing
        // comes back on re-enable.
        assert_eq!(bus.published().len(), before);
        p.apply_disabled_workers(disabled(&[])).await;
        assert!(registry.serve("state/page.js").is_none());
        assert_eq!(bus.published().len(), before);
    }

    #[tokio::test]
    async fn the_console_itself_cannot_be_disabled() {
        let bus = MockBus::new();
        bus.set_content("console/config-form.js", "v1");
        let (p, registry) = processor(bus.clone());
        p.commit_asset("o1", script("console/config-form.js")).await;

        p.apply_disabled_workers(disabled(&["console", "state"]))
            .await;

        assert!(registry.serve("console/config-form.js").is_some());
        let workers = registry.workers();
        let console = workers.iter().find(|w| w.worker == "console").unwrap();
        assert!(console.enabled);
    }

    #[tokio::test]
    async fn disabled_worker_with_no_assets_stays_listed() {
        let bus = MockBus::new();
        let (p, registry) = processor(bus.clone());
        p.apply_disabled_workers(disabled(&["ghost"])).await;
        let workers = registry.workers();
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].worker, "ghost");
        assert!(!workers[0].enabled);
        assert_eq!(workers[0].assets, 0);
    }
}
