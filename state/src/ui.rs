//! Injectable console UI for the state worker
//! (iii/tech-specs/2026-07-17-injectable-ui).
//!
//! Ships two assets into any running console:
//!
//! - `state/page.js` (`console:script`) — a state-manager page (scopes →
//!   keys → JSON value editor), plus the function-trigger renderer and the
//!   custom configuration form its `setup(host)` registers.
//! - `state/styles.css` (`console:style`) — the stylesheet, every rule
//!   scoped under `[data-iii-ui="state"]`; the console mounts it as a
//!   `<link>` and link-swaps it on change, styles-before-scripts on boot.
//!
//! 1. `state::ui-content` is the *content function*: the console worker
//!    invokes it with `{path}` and gets `{content, content_type}` back.
//! 2. One trigger per asset registered over the SDK Message path binds the
//!    path to that function. Registration IS deployment: the engine
//!    forwards it to whichever worker owns the `console:*` type, parks it
//!    while the console is down, and GCs it when this worker disconnects —
//!    injected UI dies with its worker.
//!
//! The assets are compiled from `ui/` by esbuild (react + @iii-dev/console-ui
//! external — they resolve through the console's import map at runtime) and
//! embedded at compile time. For the dev loop, set `III_STATE_UI_WATCH` to
//! the build output directory (or `1` for `ui/dist`; a `.js` file path is
//! accepted for compatibility and means its parent directory): the worker
//! polls both files' contents and re-registers a changed asset's trigger —
//! every open console tab hot-swaps it (register-first, then unregister the
//! previous handle, per the authoring contract).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub const UI_CONTENT_FUNCTION_ID: &str = "state::ui-content";
pub const PAGE_PATH: &str = "state/page.js";
pub const STYLES_PATH: &str = "state/styles.css";
const WATCH_ENV: &str = "III_STATE_UI_WATCH";
const WATCH_POLL: Duration = Duration::from_millis(1000);

/// Built by `build.rs` (esbuild over `ui/`); the embeds keep the worker a
/// single self-contained binary, like the console's own SPA.
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

/// One row per console asset this worker ships.
struct AssetSpec {
    /// Asset identity — the trigger `config.path` and `/ui/<path>` URL.
    path: &'static str,
    /// The console-owned trigger type carrying this asset kind.
    trigger_type: &'static str,
    /// MIME type served back from the content function.
    content_type: &'static str,
    /// Build output inside `ui/dist/` the dev watcher polls.
    file: &'static str,
    /// Compile-time embedded content.
    embedded: &'static str,
}

const ASSETS: [AssetSpec; 2] = [
    AssetSpec {
        path: PAGE_PATH,
        trigger_type: "console:script",
        content_type: "text/javascript; charset=utf-8",
        file: "page.js",
        embedded: PAGE_JS,
    },
    AssetSpec {
        path: STYLES_PATH,
        trigger_type: "console:style",
        content_type: "text/css; charset=utf-8",
        file: "styles.css",
        embedded: STYLES_CSS,
    },
];

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UiContentInput {
    /// The asset path from the trigger config (e.g. `state/page.js`).
    pub path: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UiContentResult {
    /// The asset source, verbatim.
    pub content: String,
    /// MIME type the console should serve the asset with.
    pub content_type: String,
}

/// Serve the assets from in-memory cells so the dev watcher can swap the
/// bytes without re-registering the function.
struct UiAssets {
    content: RwLock<HashMap<&'static str, String>>,
}

impl UiAssets {
    fn new() -> Self {
        Self {
            content: RwLock::new(
                ASSETS
                    .iter()
                    .map(|spec| (spec.path, spec.embedded.to_string()))
                    .collect(),
            ),
        }
    }

    async fn content_for(&self, path: &str) -> Result<UiContentResult, Error> {
        let spec = ASSETS.iter().find(|spec| spec.path == path).ok_or_else(|| {
            Error::Handler(format!(
                "UNKNOWN_UI_ASSET: '{path}' is not a state ui asset (expected '{PAGE_PATH}' \
                 or '{STYLES_PATH}')"
            ))
        })?;
        let content = self
            .content
            .read()
            .await
            .get(spec.path)
            .cloned()
            .unwrap_or_default();
        Ok(UiContentResult {
            content,
            content_type: spec.content_type.to_string(),
        })
    }

    async fn swap(&self, path: &'static str, next: String) {
        self.content.write().await.insert(path, next);
    }
}

/// Register the content function and one trigger per asset; spawn the dev
/// watcher when `III_STATE_UI_WATCH` is set. Call after
/// `functions::register_functions`.
pub fn register(iii: &Arc<IIIClient>) {
    let assets = Arc::new(UiAssets::new());

    {
        let assets = assets.clone();
        iii.register_function(
            UI_CONTENT_FUNCTION_ID,
            RegisterFunction::new_async(move |input: UiContentInput| {
                let assets = assets.clone();
                async move { assets.content_for(&input.path).await }
            })
            .description(
                "Serve the state worker's injected console UI assets (content function \
                 for its console:script / console:style triggers).",
            )
            // Console plumbing, not an API for agents/workers to discover.
            .metadata(serde_json::json!({ "internal": true })),
        );
    }

    let mut watched = Vec::new();
    for spec in &ASSETS {
        match register_asset_trigger(iii, spec) {
            Ok(handle) => {
                tracing::info!(path = spec.path, "registered console ui asset");
                watched.push(WatchedAsset {
                    spec,
                    handle,
                    prev: spec.embedded.to_string(),
                });
            }
            Err(e) => tracing::warn!(
                error = %e,
                path = spec.path,
                "failed to register console ui trigger"
            ),
        }
    }

    if let Some(dist) = watch_target() {
        if watched.is_empty() {
            tracing::warn!("ui watch requested but no ui trigger registered — watcher not started");
        } else {
            spawn_watcher(iii.clone(), assets, watched, dist);
        }
    }
}

fn register_asset_trigger(
    iii: &Arc<IIIClient>,
    spec: &AssetSpec,
) -> Result<iii_sdk::trigger::Trigger, Error> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: spec.trigger_type.to_string(),
        function_id: UI_CONTENT_FUNCTION_ID.to_string(),
        config: serde_json::json!({ "path": spec.path }),
        metadata: None,
    })
}

fn watch_target() -> Option<PathBuf> {
    let raw = std::env::var(WATCH_ENV).ok()?;
    if raw.is_empty() || raw == "0" || raw.eq_ignore_ascii_case("false") {
        return None;
    }
    if raw == "1" || raw.eq_ignore_ascii_case("true") {
        return Some(PathBuf::from("ui/dist"));
    }
    let path = PathBuf::from(raw);
    // Pre-styles compatibility: the env var used to name the built page.js.
    if path.extension().is_some_and(|ext| ext == "js") {
        return Some(path.parent().map(PathBuf::from).unwrap_or_default());
    }
    Some(path)
}

struct WatchedAsset {
    spec: &'static AssetSpec,
    handle: iii_sdk::trigger::Trigger,
    prev: String,
}

/// Dev-loop hot reload: poll the built files; on change, swap the served
/// bytes, register a fresh trigger for the same path (the console
/// supersedes + re-fetches + pushes to every tab), THEN unregister the
/// previous handle — register-first avoids a zero-trigger flash, and the
/// explicit unregister keeps the SDK's replay map at one entry per path.
fn spawn_watcher(
    iii: Arc<IIIClient>,
    assets: Arc<UiAssets>,
    mut watched: Vec<WatchedAsset>,
    dist: PathBuf,
) {
    tokio::spawn(async move {
        tracing::info!(dir = %dist.display(), "ui watch enabled — hot reload on rebuild");
        loop {
            tokio::time::sleep(WATCH_POLL).await;
            for asset in watched.iter_mut() {
                let file = dist.join(asset.spec.file);
                let Ok(next) = tokio::fs::read_to_string(&file).await else {
                    continue;
                };
                if next == asset.prev {
                    continue;
                }
                assets.swap(asset.spec.path, next.clone()).await;
                asset.prev = next;
                match register_asset_trigger(&iii, asset.spec) {
                    Ok(next_handle) => {
                        let old = std::mem::replace(&mut asset.handle, next_handle);
                        old.unregister();
                        tracing::info!(
                            path = asset.spec.path,
                            "ui asset re-registered (hot reload)"
                        );
                    }
                    Err(e) => tracing::warn!(
                        error = %e,
                        path = asset.spec.path,
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

    #[tokio::test]
    async fn serves_the_page_asset() {
        let result = UiAssets::new().content_for(PAGE_PATH).await.unwrap();
        assert_eq!(result.content, PAGE_JS);
        assert!(result.content_type.starts_with("text/javascript"));
    }

    #[tokio::test]
    async fn serves_the_styles_asset() {
        let result = UiAssets::new().content_for(STYLES_PATH).await.unwrap();
        assert_eq!(result.content, STYLES_CSS);
        assert!(result.content_type.starts_with("text/css"));
    }

    #[tokio::test]
    async fn swap_replaces_served_bytes() {
        let assets = UiAssets::new();
        assets.swap(PAGE_PATH, "export default () => {}".to_string()).await;
        let result = assets.content_for(PAGE_PATH).await.unwrap();
        assert_eq!(result.content, "export default () => {}");
    }

    #[tokio::test]
    async fn unknown_path_errors() {
        let err = UiAssets::new().content_for("state/nope.js").await.unwrap_err();
        assert!(err.to_string().contains("UNKNOWN_UI_ASSET"));
    }

    #[test]
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=state]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="state"]"#)
                || STYLES_CSS.contains("[data-iii-ui=state]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }

    #[test]
    fn asset_paths_match_their_trigger_types() {
        for spec in &ASSETS {
            match spec.trigger_type {
                "console:script" => assert!(spec.path.ends_with(".js")),
                "console:style" => assert!(spec.path.ends_with(".css")),
                other => panic!("unexpected trigger type {other}"),
            }
        }
    }

    #[test]
    fn watch_target_parses_env_conventions() {
        // Not set in tests by default — None.
        assert!(watch_target().is_none() || std::env::var(WATCH_ENV).is_ok());
    }
}
