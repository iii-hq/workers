//! Worker-side helper for the injectable console UI
//! (`iii/tech-specs/2026-07-17-injectable-ui`; authoring guide:
//! `workers/docs/sops/injectable-console-ui.md`).
//!
//! One builder owns the whole worker-side registration contract so an author
//! cannot get it subtly wrong:
//!
//! - the *content function* (`<worker>::ui-content` by default) the console
//!   calls through each asset's observer with `{path}` to fetch its source,
//! - one observer of `console::ui::asset` per asset, with args
//!   `{ kind: "script" | "style" | "module", path }` and the content function
//!   as its target (the SDK replays it by id after a reconnect, and the
//!   engine disposes it once the worker stays disconnected). Modules and
//!   styles are observed at once; a script is observed once every module
//!   observer is active or has failed, so the console commits the modules a
//!   script imports before the script. After a reconnect the SDK replays the
//!   observers in its own order,
//! - the dev-loop hot-reload watcher (`III_<WORKER>_UI_WATCH` by default):
//!   poll the build output, swap the served bytes, observe the same path
//!   AFRESH, THEN dispose the previous observer — observe-first avoids a
//!   flash without the asset in open tabs (the console supersedes the old
//!   observer), and the trailing dispose keeps the SDK's replay map at one
//!   observer per path.
//!
//! ```rust,ignore
//! use iii_console_ui::ConsoleUi;
//!
//! ConsoleUi::new("state")
//!     .script("state/page.js", include_str!("../ui/dist/page.js"))
//!     .style("state/styles.css", include_str!("../ui/dist/styles.css"))
//!     .register(&iii);
//! ```
//!
//! The builder panics on wire-contract violations (bad path, extension/kind
//! mismatch, duplicate path) — the console would reject the registration
//! anyway, and a panic at first boot or in a unit test is a far shorter
//! feedback loop than a warn-log on a running engine.
//!
//! This crate is linked by path (`iii-console-ui = { path =
//! "../crates/console-ui" }`) and deliberately not published: it versions
//! with the console worker in this repo, not with the SDK.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, ObserverHandle, ObserverState, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Empty user request. Ignore transport metadata such as `_caller_worker_id`,
/// which the engine adds to routed calls; no payload field selects the entry.
#[derive(Debug, Deserialize, JsonSchema)]
struct ConfigurationIdentityRequest {}

/// Only the entry ID is exposed; configuration values and secrets stay in the service.
#[derive(Debug, Serialize, JsonSchema)]
struct ConfigurationIdentityResponse {
    id: String,
}

/// Let consumers resolve the entry through the worker they actually address,
/// not a global form-family search or a reimplementation of Compose's hash.
/// The function inherits the worker's namespace; configuration RPCs still go
/// to `default`. `id` must be the same process-stable ID used at registration.
pub fn register_configuration_identity(iii: &IIIClient, worker: &str, id: &'static str) {
    iii.register_function(
        format!("{worker}::configuration-id"),
        RegisterFunction::new(move |_request: ConfigurationIdentityRequest| {
            Ok::<_, Error>(ConfigurationIdentityResponse { id: id.to_string() })
        })
        .description(
            "Returns this worker instance's effective configuration entry ID, without its value.",
        )
        .metadata(serde_json::json!({ "internal": true })),
    );
}

/// The console's asset function: one observer per asset, its args naming the
/// asset and its target the content function.
const ASSET_FN: &str = "console::ui::asset";
const WATCH_POLL: Duration = Duration::from_millis(1000);
/// Console-enforced ceiling on the `path` arg's length.
const MAX_PATH_LEN: usize = 512;

/// Input of the content function: the console asks for one asset by path.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UiContentInput {
    /// The asset path from the observer's args (e.g. `state/page.js`).
    pub path: String,
}

/// Output of the content function.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UiContentResult {
    /// The asset source, verbatim.
    pub content: String,
    /// MIME type the console should serve the asset with.
    pub content_type: String,
}

/// The asset kinds the console accepts, with everything they imply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Script,
    Style,
    Module,
}

impl AssetKind {
    /// The `kind` arg of the asset's `console::ui::asset` observer.
    fn as_str(self) -> &'static str {
        match self {
            AssetKind::Script => "script",
            AssetKind::Style => "style",
            AssetKind::Module => "module",
        }
    }

    fn content_type(self) -> &'static str {
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
}

struct AssetSpec {
    /// Asset identity — the observer's `path` arg and `/ui/<path>` URL.
    path: String,
    kind: AssetKind,
    /// File inside the watch dir the dev watcher polls (last path segment).
    file: String,
    /// Initial (typically compile-time embedded) content.
    content: String,
}

/// Builder for a worker's injected console UI. See the crate docs for the
/// full lifecycle; construct with [`ConsoleUi::new`], add assets, then
/// [`register`](ConsoleUi::register).
pub struct ConsoleUi {
    worker: String,
    content_function_id: String,
    watch_env: String,
    watch_default_dir: PathBuf,
    assets: Vec<AssetSpec>,
}

impl ConsoleUi {
    /// Start a builder for `worker` (the worker's registry name, e.g.
    /// `state`). Derives the defaults: content function
    /// `<worker>::ui-content`, watch env `III_<WORKER>_UI_WATCH`, watch dir
    /// `ui/dist` (relative to the worker process cwd).
    pub fn new(worker: impl Into<String>) -> Self {
        let worker = worker.into();
        assert!(!worker.is_empty(), "worker name must not be empty");
        let env_stem: String = worker
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            content_function_id: format!("{worker}::ui-content"),
            watch_env: format!("III_{env_stem}_UI_WATCH"),
            watch_default_dir: PathBuf::from("ui/dist"),
            worker,
            assets: Vec::new(),
        }
    }

    /// Add a `script` asset (an ES module the console tabs
    /// `import()` and call `setup(host)` on). `content` is the initial
    /// source — typically `include_str!` of the build output.
    ///
    /// Panics on a path the console would reject (see crate docs).
    pub fn script(self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.asset(AssetKind::Script, path.into(), content.into())
    }

    /// Add a `style` asset (a stylesheet the console tabs mount as a
    /// `<link>`); every rule must be scoped under `[data-iii-ui="<worker>"]`.
    ///
    /// Panics on a path the console would reject (see crate docs).
    pub fn style(self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.asset(AssetKind::Style, path.into(), content.into())
    }

    /// Add a `module` asset: an ES module the console serves but
    /// never imports at mount. A script loads it on demand with
    /// `host.importModule(path)`, so heavy code that only one view needs
    /// (a terminal emulator, a diagram renderer) stays out of every tab's
    /// first load.
    ///
    /// Panics on a path the console would reject (see crate docs).
    pub fn module(self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.asset(AssetKind::Module, path.into(), content.into())
    }

    /// Override the content function id (default `<worker>::ui-content`).
    pub fn content_function_id(mut self, id: impl Into<String>) -> Self {
        self.content_function_id = id.into();
        self
    }

    /// Override the hot-reload env var name (default `III_<WORKER>_UI_WATCH`).
    pub fn watch_env(mut self, name: impl Into<String>) -> Self {
        self.watch_env = name.into();
        self
    }

    /// Override the directory the watcher polls when the env var is `1`/
    /// `true` rather than an explicit path (default `ui/dist`).
    pub fn watch_default_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.watch_default_dir = dir.into();
        self
    }

    fn asset(mut self, kind: AssetKind, path: String, content: String) -> Self {
        validate_path(&path, kind);
        assert!(
            !self.assets.iter().any(|a| a.path == path),
            "duplicate ui asset path '{path}'"
        );
        let file = path
            .rsplit('/')
            .next()
            .expect("validate_path guarantees a non-empty last segment")
            .to_string();
        self.assets.push(AssetSpec {
            path,
            kind,
            file,
            content,
        });
        self
    }

    /// Register the content function and observe `console::ui::asset` once
    /// per asset; spawn the dev watcher when the watch env var is set. Call
    /// once, after the worker's regular functions are registered, inside a
    /// tokio runtime.
    ///
    /// Modules and styles are observed at once. When the worker has modules,
    /// each script is observed once every module observer is active or has
    /// failed: the console commits assets in the order their observers
    /// arrive, so it serves the modules a script imports before the script
    /// reaches a tab. The order holds whether `register` runs before or after
    /// the connection is up and whether the console registers before or
    /// after this worker.
    ///
    /// Each observer stays `pending` until the console registers the asset
    /// function, and the SDK replays it on reconnect. A rejection (an asset
    /// the console refuses, a failed content fetch) is warn-logged, not
    /// fatal — injected UI is an accessory.
    pub fn register(self, iii: &Arc<IIIClient>) {
        let Self {
            worker,
            content_function_id,
            watch_env,
            watch_default_dir,
            assets,
        } = self;
        if assets.is_empty() {
            tracing::warn!(
                worker,
                "ConsoleUi::register called with no assets — nothing to do"
            );
            return;
        }
        for spec in &assets {
            if spec.path.split('/').next() != Some(worker.as_str()) {
                // Convention, not wire contract: the first segment becomes
                // the data-iii-ui style scope and the only attribution.
                tracing::warn!(
                    worker,
                    path = spec.path,
                    "ui asset path does not start with '<worker>/' — style scoping and \
                     attribution will not point at this worker"
                );
            }
        }

        let served = Arc::new(Served::new(&assets));

        {
            let served = served.clone();
            iii.register_function(
                content_function_id.clone(),
                RegisterFunction::new_async(move |input: UiContentInput| {
                    let served = served.clone();
                    async move { served.content_for(&input.path).await }
                })
                .description(format!(
                    "Serve the {worker} worker's injected console UI assets (the target of \
                     its console::ui::asset observers; the console calls it through each \
                     asset's observer with {{path}})."
                ))
                // Console plumbing, not an API for agents/workers to discover.
                .metadata(serde_json::json!({ "internal": true })),
            );
        }

        let dist = watch_target(&watch_env, &watch_default_dir);
        let (first, scripts) = observe_plan(assets);
        let mut watched: Vec<WatchedAsset> = first
            .into_iter()
            .map(|spec| observe_watched(iii, &content_function_id, spec))
            .collect();
        let modules: Vec<ObserverHandle> = watched
            .iter()
            .filter(|asset| asset.kind == AssetKind::Module)
            .map(|asset| asset.handle.clone())
            .collect();

        if modules.is_empty() || scripts.is_empty() {
            watched.extend(
                scripts
                    .into_iter()
                    .map(|spec| observe_watched(iii, &content_function_id, spec)),
            );
            if let Some(dist) = dist {
                spawn_watcher(iii.clone(), served, content_function_id, watched, dist);
            }
            return;
        }

        let iii = iii.clone();
        tokio::spawn(async move {
            for module in &modules {
                // `ready` fails once the observer is in `error` or
                // `disposed`; either way it has settled, and the scripts go.
                let _ = module.ready().await;
            }
            watched.extend(
                scripts
                    .into_iter()
                    .map(|spec| observe_watched(&iii, &content_function_id, spec)),
            );
            if let Some(dist) = dist {
                spawn_watcher(iii, served, content_function_id, watched, dist);
            }
        });
    }
}

/// The order `register` observes the assets in: the first list (modules,
/// then styles) at once, the second (scripts) once every module observer has
/// settled. Each list keeps the order the builder added the assets in.
fn observe_plan(assets: Vec<AssetSpec>) -> (Vec<AssetSpec>, Vec<AssetSpec>) {
    let (scripts, mut first): (Vec<_>, Vec<_>) = assets
        .into_iter()
        .partition(|spec| spec.kind == AssetKind::Script);
    first.sort_by_key(|spec| spec.kind != AssetKind::Module);
    (first, scripts)
}

/// Observe one asset and keep what the dev watcher needs to reload it.
fn observe_watched(iii: &Arc<IIIClient>, function_id: &str, spec: AssetSpec) -> WatchedAsset {
    let handle = observe_asset(iii, function_id, spec.kind, &spec.path);
    tracing::info!(
        path = spec.path,
        observer_id = handle.id(),
        "observing console ui asset"
    );
    WatchedAsset {
        path: spec.path,
        kind: spec.kind,
        file: spec.file,
        handle,
        prev: spec.content,
    }
}

/// Enforce the console's path rules at build time — the console rejects the
/// registration otherwise, with a much slower feedback loop.
fn validate_path(path: &str, kind: AssetKind) {
    assert!(!path.is_empty(), "ui asset path must not be empty");
    assert!(
        path.len() <= MAX_PATH_LEN,
        "ui asset path '{path}' exceeds {MAX_PATH_LEN} chars"
    );
    assert!(
        !path.starts_with('/'),
        "ui asset path '{path}' must not start with '/'"
    );
    for segment in path.split('/') {
        assert!(
            !segment.is_empty(),
            "ui asset path '{path}' has an empty segment"
        );
        assert!(
            segment != "." && segment != "..",
            "ui asset path '{path}' must not contain '.' or '..' segments"
        );
        assert!(
            segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c)),
            "ui asset path '{path}' has characters outside [a-z0-9._-] in segment '{segment}'"
        );
    }
    assert!(
        path.ends_with(kind.extension()),
        "ui asset path '{path}' must end with '{}' for a {} asset",
        kind.extension(),
        kind.as_str()
    );
}

/// Serve the assets from in-memory cells so the dev watcher can swap the
/// bytes without re-registering the function.
struct Served {
    content: RwLock<HashMap<String, ServedAsset>>,
    /// Sorted paths, for the unknown-path error message.
    known: Vec<String>,
}

struct ServedAsset {
    content: String,
    content_type: &'static str,
}

impl Served {
    fn new(assets: &[AssetSpec]) -> Self {
        let mut known: Vec<String> = assets.iter().map(|a| a.path.clone()).collect();
        known.sort();
        Self {
            content: RwLock::new(
                assets
                    .iter()
                    .map(|a| {
                        (
                            a.path.clone(),
                            ServedAsset {
                                content: a.content.clone(),
                                content_type: a.kind.content_type(),
                            },
                        )
                    })
                    .collect(),
            ),
            known,
        }
    }

    async fn content_for(&self, path: &str) -> Result<UiContentResult, Error> {
        let guard = self.content.read().await;
        let asset = guard.get(path).ok_or_else(|| {
            Error::Handler(format!(
                "UNKNOWN_UI_ASSET: '{path}' is not one of this worker's ui assets \
                 (expected one of: {})",
                self.known.join(", ")
            ))
        })?;
        Ok(UiContentResult {
            content: asset.content.clone(),
            content_type: asset.content_type.to_string(),
        })
    }

    async fn swap(&self, path: &str, next: String) {
        if let Some(asset) = self.content.write().await.get_mut(path) {
            asset.content = next;
        }
    }
}

/// Observe `console::ui::asset` for one asset, with the content function as
/// the target. A rejection is logged when it arrives.
fn observe_asset(
    iii: &Arc<IIIClient>,
    function_id: &str,
    kind: AssetKind,
    path: &str,
) -> ObserverHandle {
    let handle = iii.observe(
        Source::function(ASSET_FN).args(asset_args(kind, path)),
        function_id,
    );
    let path = path.to_string();
    // The listener lives as long as the observer; the returned token only
    // unsubscribes it.
    let _listener = handle.on_state_change(move |state, reason| {
        if *state != ObserverState::Error {
            return;
        }
        let (code, message) = reason
            .map(|r| (r.code.as_str(), r.message.as_str()))
            .unwrap_or(("OBSERVER_ERROR", ""));
        if code == "UI_ASSET_SUPERSEDED" {
            // Hot reload observes the path afresh before disposing this one.
            tracing::debug!(path = %path, "console ui asset superseded");
        } else {
            tracing::warn!(path = %path, code, message, "console ui asset rejected");
        }
    });
    handle
}

/// The args of an asset's `console::ui::asset` observer.
fn asset_args(kind: AssetKind, path: &str) -> serde_json::Value {
    serde_json::json!({ "kind": kind.as_str(), "path": path })
}

fn watch_target(env: &str, default_dir: &Path) -> Option<PathBuf> {
    parse_watch_target(&std::env::var(env).ok()?, default_dir)
}

fn parse_watch_target(raw: &str, default_dir: &Path) -> Option<PathBuf> {
    if raw.is_empty() || raw == "0" || raw.eq_ignore_ascii_case("false") {
        return None;
    }
    if raw == "1" || raw.eq_ignore_ascii_case("true") {
        return Some(default_dir.to_path_buf());
    }
    let path = PathBuf::from(raw);
    // Compatibility: the env var may name a built .js file and means its
    // parent directory.
    if path.extension().is_some_and(|ext| ext == "js") {
        return Some(path.parent().map(PathBuf::from).unwrap_or_default());
    }
    Some(path)
}

struct WatchedAsset {
    path: String,
    kind: AssetKind,
    file: String,
    handle: ObserverHandle,
    prev: String,
}

/// Dev-loop hot reload: poll the built files; on change, swap the served
/// bytes, observe the same path afresh (the console supersedes the old
/// observer, re-fetches, and pushes to every tab), THEN dispose the previous
/// observer.
fn spawn_watcher(
    iii: Arc<IIIClient>,
    served: Arc<Served>,
    function_id: String,
    mut watched: Vec<WatchedAsset>,
    dist: PathBuf,
) {
    tokio::spawn(async move {
        tracing::info!(dir = %dist.display(), "ui watch enabled — hot reload on rebuild");
        loop {
            tokio::time::sleep(WATCH_POLL).await;
            for asset in watched.iter_mut() {
                let file = dist.join(&asset.file);
                let Ok(next) = tokio::fs::read_to_string(&file).await else {
                    continue;
                };
                if next == asset.prev {
                    continue;
                }
                served.swap(&asset.path, next.clone()).await;
                asset.prev = next;
                let next_handle = observe_asset(&iii, &function_id, asset.kind, &asset.path);
                let old = std::mem::replace(&mut asset.handle, next_handle);
                if let Err(e) = old.dispose().await {
                    tracing::warn!(
                        error = %e,
                        path = asset.path,
                        "ui hot-reload dispose of the previous observer failed"
                    );
                }
                tracing::info!(path = asset.path, "ui asset observed afresh (hot reload)");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identity RPC accepts engine metadata without making its entry ID caller-controlled.
    #[test]
    fn configuration_identity_accepts_engine_caller_metadata() {
        for payload in [
            serde_json::json!({}),
            serde_json::json!({
                "_caller_worker_id": "00000000-0000-4000-8000-000000000002"
            }),
        ] {
            serde_json::from_value::<ConfigurationIdentityRequest>(payload)
                .expect("routed identity requests accept engine metadata");
        }
    }

    fn two_assets() -> ConsoleUi {
        ConsoleUi::new("demo")
            .script("demo/page.js", "export default () => {}")
            .style("demo/styles.css", "[data-iii-ui=demo] a { color: red }")
    }

    #[test]
    fn defaults_derive_from_the_worker_name() {
        let ui = ConsoleUi::new("state");
        assert_eq!(ui.content_function_id, "state::ui-content");
        assert_eq!(ui.watch_env, "III_STATE_UI_WATCH");
        assert_eq!(ui.watch_default_dir, PathBuf::from("ui/dist"));

        let ui = ConsoleUi::new("image-resize");
        assert_eq!(ui.content_function_id, "image-resize::ui-content");
        assert_eq!(ui.watch_env, "III_IMAGE_RESIZE_UI_WATCH");
    }

    #[test]
    fn overrides_replace_the_defaults() {
        let ui = ConsoleUi::new("demo")
            .content_function_id("demo::assets")
            .watch_env("DEMO_WATCH")
            .watch_default_dir("build/out");
        assert_eq!(ui.content_function_id, "demo::assets");
        assert_eq!(ui.watch_env, "DEMO_WATCH");
        assert_eq!(ui.watch_default_dir, PathBuf::from("build/out"));
    }

    #[test]
    fn assets_record_kind_and_watch_file() {
        let ui = two_assets();
        assert_eq!(ui.assets.len(), 2);
        assert_eq!(ui.assets[0].kind.as_str(), "script");
        assert_eq!(ui.assets[0].file, "page.js");
        assert_eq!(ui.assets[1].kind.as_str(), "style");
        assert_eq!(ui.assets[1].file, "styles.css");
    }

    #[test]
    fn each_asset_observes_the_asset_function_with_kind_and_path_args() {
        assert_eq!(ASSET_FN, "console::ui::asset");
        assert_eq!(
            asset_args(AssetKind::Script, "demo/page.js"),
            serde_json::json!({ "kind": "script", "path": "demo/page.js" })
        );
        assert_eq!(
            asset_args(AssetKind::Style, "demo/styles.css"),
            serde_json::json!({ "kind": "style", "path": "demo/styles.css" })
        );
        assert_eq!(
            asset_args(AssetKind::Module, "demo/heavy.js"),
            serde_json::json!({ "kind": "module", "path": "demo/heavy.js" })
        );
    }

    #[tokio::test]
    async fn serves_registered_assets_with_content_types() {
        let served = Served::new(&two_assets().assets);
        let page = served.content_for("demo/page.js").await.unwrap();
        assert_eq!(page.content, "export default () => {}");
        assert!(page.content_type.starts_with("text/javascript"));
        let styles = served.content_for("demo/styles.css").await.unwrap();
        assert!(styles.content_type.starts_with("text/css"));
    }

    #[tokio::test]
    async fn swap_replaces_served_bytes() {
        let served = Served::new(&two_assets().assets);
        served
            .swap("demo/page.js", "export const v2 = 1".to_string())
            .await;
        let page = served.content_for("demo/page.js").await.unwrap();
        assert_eq!(page.content, "export const v2 = 1");
    }

    #[tokio::test]
    async fn unknown_path_errors_and_names_the_known_paths() {
        let err = Served::new(&two_assets().assets)
            .content_for("demo/nope.js")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("UNKNOWN_UI_ASSET"));
        assert!(msg.contains("demo/page.js"));
        assert!(msg.contains("demo/styles.css"));
    }

    #[test]
    fn parse_watch_target_conventions() {
        let dist = Path::new("ui/dist");
        assert_eq!(parse_watch_target("", dist), None);
        assert_eq!(parse_watch_target("0", dist), None);
        assert_eq!(parse_watch_target("false", dist), None);
        assert_eq!(parse_watch_target("FALSE", dist), None);
        assert_eq!(
            parse_watch_target("1", dist),
            Some(PathBuf::from("ui/dist"))
        );
        assert_eq!(
            parse_watch_target("true", dist),
            Some(PathBuf::from("ui/dist"))
        );
        assert_eq!(
            parse_watch_target("build/out", dist),
            Some(PathBuf::from("build/out"))
        );
        // A .js file path means its parent directory (pre-crate compatibility).
        assert_eq!(
            parse_watch_target("build/out/page.js", dist),
            Some(PathBuf::from("build/out"))
        );
    }

    #[test]
    #[should_panic(expected = "must end with '.js'")]
    fn script_with_css_extension_panics() {
        let _ = ConsoleUi::new("demo").script("demo/styles.css", "");
    }

    #[test]
    #[should_panic(expected = "must end with '.js'")]
    fn module_with_css_extension_panics() {
        let _ = ConsoleUi::new("demo").module("demo/styles.css", "");
    }

    #[tokio::test]
    async fn modules_observe_with_their_own_kind_and_serve_as_javascript() {
        let ui = two_assets().module("demo/heavy.js", "export const x = 1");
        assert_eq!(ui.assets[2].kind.as_str(), "module");
        assert_eq!(ui.assets[2].file, "heavy.js");
        let served = Served::new(&ui.assets);
        let heavy = served.content_for("demo/heavy.js").await.unwrap();
        assert!(heavy.content_type.starts_with("text/javascript"));
    }

    fn paths(assets: &[AssetSpec]) -> Vec<&str> {
        assets.iter().map(|a| a.path.as_str()).collect()
    }

    #[test]
    fn observe_plan_observes_modules_then_styles_and_holds_scripts_back() {
        let ui = ConsoleUi::new("demo")
            .script("demo/page.js", "")
            .style("demo/styles.css", "")
            .module("demo/heavy.js", "")
            .script("demo/other.js", "")
            .module("demo/diagram.js", "");
        let (first, scripts) = observe_plan(ui.assets);
        assert_eq!(
            paths(&first),
            ["demo/heavy.js", "demo/diagram.js", "demo/styles.css"]
        );
        assert_eq!(paths(&scripts), ["demo/page.js", "demo/other.js"]);
    }

    #[test]
    fn observe_plan_without_modules_keeps_styles_first_and_scripts_second() {
        let (first, scripts) = observe_plan(two_assets().assets);
        assert_eq!(paths(&first), ["demo/styles.css"]);
        assert_eq!(paths(&scripts), ["demo/page.js"]);
    }

    #[test]
    #[should_panic(expected = "must end with '.css'")]
    fn style_with_js_extension_panics() {
        let _ = ConsoleUi::new("demo").style("demo/page.js", "");
    }

    #[test]
    #[should_panic(expected = "outside [a-z0-9._-]")]
    fn uppercase_path_panics() {
        let _ = ConsoleUi::new("demo").script("demo/Page.js", "");
    }

    #[test]
    #[should_panic(expected = "'.' or '..'")]
    fn dotdot_segment_panics() {
        let _ = ConsoleUi::new("demo").script("demo/../page.js", "");
    }

    #[test]
    #[should_panic(expected = "must not start with '/'")]
    fn leading_slash_panics() {
        let _ = ConsoleUi::new("demo").script("/demo/page.js", "");
    }

    #[test]
    #[should_panic(expected = "duplicate ui asset path")]
    fn duplicate_path_panics() {
        let _ = ConsoleUi::new("demo")
            .script("demo/page.js", "a")
            .script("demo/page.js", "b");
    }
}
