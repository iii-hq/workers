//! Worker-side helper for the injectable console UI
//! (`iii/tech-specs/2026-07-17-injectable-ui`; authoring guide:
//! `workers/docs/sops/injectable-console-ui.md`).
//!
//! One builder owns the whole worker-side registration contract so an author
//! cannot get it subtly wrong:
//!
//! - the *content function* (`<worker>::ui-content` by default) the console
//!   invokes with `{path}` to fetch an asset's source,
//! - one `console:script` / `console:style` trigger per asset, registered
//!   over the SDK Message path (disconnect GC + reconnect replay — never
//!   `engine::register_trigger`),
//! - the dev-loop hot-reload watcher (`III_<WORKER>_UI_WATCH` by default):
//!   poll the build output, swap the served bytes, register a FRESH trigger
//!   for the same path, THEN unregister the previous handle — register-first
//!   avoids a zero-trigger flash in open tabs, and the trailing unregister
//!   keeps the SDK's reconnect-replay map at one entry per path.
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
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const WATCH_POLL: Duration = Duration::from_millis(1000);
/// Console-enforced ceiling on `config.path` length.
const MAX_PATH_LEN: usize = 512;

/// Input of the content function: the console asks for one asset by path.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UiContentInput {
    /// The asset path from the trigger config (e.g. `state/page.js`).
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

/// The two asset kinds the console accepts, with everything they imply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Script,
    Style,
}

impl AssetKind {
    fn trigger_type(self) -> &'static str {
        match self {
            AssetKind::Script => "console:script",
            AssetKind::Style => "console:style",
        }
    }

    fn content_type(self) -> &'static str {
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
}

struct AssetSpec {
    /// Asset identity — the trigger `config.path` and `/ui/<path>` URL.
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

    /// Add a `console:script` asset (an ES module the console tabs
    /// `import()` and call `setup(host)` on). `content` is the initial
    /// source — typically `include_str!` of the build output.
    ///
    /// Panics on a path the console would reject (see crate docs).
    pub fn script(self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.asset(AssetKind::Script, path.into(), content.into())
    }

    /// Add a `console:style` asset (a stylesheet the console tabs mount as a
    /// `<link>`); every rule must be scoped under `[data-iii-ui="<worker>"]`.
    ///
    /// Panics on a path the console would reject (see crate docs).
    pub fn style(self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.asset(AssetKind::Style, path.into(), content.into())
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

    /// Register the content function and one trigger per asset; spawn the
    /// dev watcher when the watch env var is set. Call once, after the
    /// worker's regular functions are registered.
    ///
    /// Trigger registration failures are warn-logged, not fatal — injected
    /// UI is an accessory, and the SDK replays surviving registrations on
    /// reconnect.
    pub fn register(self, iii: &Arc<IIIClient>) {
        if self.assets.is_empty() {
            tracing::warn!(
                worker = self.worker,
                "ConsoleUi::register called with no assets — nothing to do"
            );
            return;
        }
        for spec in &self.assets {
            if spec.path.split('/').next() != Some(self.worker.as_str()) {
                // Convention, not wire contract: the first segment becomes
                // the data-iii-ui style scope and the only attribution.
                tracing::warn!(
                    worker = self.worker,
                    path = spec.path,
                    "ui asset path does not start with '<worker>/' — style scoping and \
                     attribution will not point at this worker"
                );
            }
        }

        let served = Arc::new(Served::new(&self.assets));

        {
            let served = served.clone();
            iii.register_function(
                self.content_function_id.clone(),
                RegisterFunction::new_async(move |input: UiContentInput| {
                    let served = served.clone();
                    async move { served.content_for(&input.path).await }
                })
                .description(format!(
                    "Serve the {} worker's injected console UI assets (content function \
                     for its console:script / console:style triggers).",
                    self.worker
                ))
                // Console plumbing, not an API for agents/workers to discover.
                .metadata(serde_json::json!({ "internal": true })),
            );
        }

        let mut watched = Vec::new();
        for spec in &self.assets {
            match register_asset_trigger(iii, &self.content_function_id, spec.kind, &spec.path) {
                Ok(handle) => {
                    tracing::info!(path = spec.path, "registered console ui asset");
                    watched.push(WatchedAsset {
                        path: spec.path.clone(),
                        kind: spec.kind,
                        file: spec.file.clone(),
                        handle,
                        prev: spec.content.clone(),
                    });
                }
                Err(e) => tracing::warn!(
                    error = %e,
                    path = spec.path,
                    "failed to register console ui trigger"
                ),
            }
        }

        if let Some(dist) = watch_target(&self.watch_env, &self.watch_default_dir) {
            if watched.is_empty() {
                tracing::warn!(
                    "ui watch requested but no ui trigger registered — watcher not started"
                );
            } else {
                spawn_watcher(iii.clone(), served, self.content_function_id, watched, dist);
            }
        }
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
        kind.trigger_type()
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

fn register_asset_trigger(
    iii: &Arc<IIIClient>,
    function_id: &str,
    kind: AssetKind,
    path: &str,
) -> Result<iii_sdk::trigger::Trigger, Error> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: kind.trigger_type().to_string(),
        function_id: function_id.to_string(),
        config: serde_json::json!({ "path": path }),
        metadata: None,
    })
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
    handle: iii_sdk::trigger::Trigger,
    prev: String,
}

/// Dev-loop hot reload: poll the built files; on change, swap the served
/// bytes, register a fresh trigger for the same path (the console supersedes
/// + re-fetches + pushes to every tab), THEN unregister the previous handle.
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
                match register_asset_trigger(&iii, &function_id, asset.kind, &asset.path) {
                    Ok(next_handle) => {
                        let old = std::mem::replace(&mut asset.handle, next_handle);
                        old.unregister();
                        tracing::info!(path = asset.path, "ui asset re-registered (hot reload)");
                    }
                    Err(e) => tracing::warn!(
                        error = %e,
                        path = asset.path,
                        "ui hot-reload re-register failed"
                    ),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(ui.assets[0].kind.trigger_type(), "console:script");
        assert_eq!(ui.assets[0].file, "page.js");
        assert_eq!(ui.assets[1].kind.trigger_type(), "console:style");
        assert_eq!(ui.assets[1].file, "styles.css");
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
