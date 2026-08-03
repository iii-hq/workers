//! Injectable console UI for the harness worker
//! (iii/tech-specs/2026-07-17-injectable-ui; authoring SOP:
//! workers/docs/sops/injectable-console-ui.md).
//!
//! Ships two assets into any running console:
//!
//! - `harness/page.js` (`console:script`) — the `context` session chip
//!   (live context-window usage with a category-breakdown popover; the
//!   `chat.registerSessionChip` slot is feature-detected, older consoles
//!   skip it) plus the `harness::metrics` function-trigger renderer its
//!   `setup(host)` registers.
//! - `harness/styles.css` (`console:style`) — the stylesheet, every rule
//!   scoped under `[data-iii-ui="harness"]`; the console mounts it as a
//!   `<link>` and link-swaps it on change, styles-before-scripts on boot.
//!
//! Other workers get the registration machinery from the shared
//! `iii-console-ui` crate (`workers/crates/console-ui`), but that crate
//! pins `iii-sdk = "=0.21.6"` while the harness needs `=0.21.8` (the
//! reconnect reattach handshake) — two semver-compatible exact pins cargo
//! cannot co-resolve. This module therefore implements the same wire
//! contract against the harness's own SDK: the content function
//! `harness::ui-content` (`{path}` in, `{content, content_type}` out,
//! flagged internal), one Message-path trigger per asset (never
//! `engine::register_trigger`), and the `III_HARNESS_UI_WATCH` dev poller
//! (swap the served bytes, register a FRESH trigger for the same path,
//! THEN unregister the previous handle).
//!
//! The assets are compiled from `ui/` by esbuild (react + @iii-dev/console-ui
//! external — they resolve through the console's import map at runtime) and
//! embedded at compile time so the worker stays one self-contained binary.
//! For the dev loop, set `III_HARNESS_UI_WATCH` to the build output
//! directory (or `1` for `ui/dist`): the worker polls both files and
//! re-registers a changed asset's trigger — every open console tab
//! hot-swaps it.

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

pub const PAGE_PATH: &str = "harness/page.js";
pub const STYLES_PATH: &str = "harness/styles.css";

const CONTENT_FN: &str = "harness::ui-content";
const WATCH_ENV: &str = "III_HARNESS_UI_WATCH";
const WATCH_DEFAULT_DIR: &str = "ui/dist";
const WATCH_POLL: Duration = Duration::from_millis(1000);

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

/// Input of the content function: the console asks for one asset by path.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UiContentInput {
    pub path: String,
}

/// Output of the content function.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UiContentResult {
    pub content: String,
    pub content_type: String,
}

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
}

struct AssetSpec {
    path: &'static str,
    kind: AssetKind,
    /// File inside the watch dir the dev poller reads (last path segment).
    file: &'static str,
    content: &'static str,
}

const ASSETS: [AssetSpec; 2] = [
    AssetSpec {
        path: PAGE_PATH,
        kind: AssetKind::Script,
        file: "page.js",
        content: PAGE_JS,
    },
    AssetSpec {
        path: STYLES_PATH,
        kind: AssetKind::Style,
        file: "styles.css",
        content: STYLES_CSS,
    },
];

/// Register the harness's console UI: the content function plus one
/// Message-path trigger per asset, and the dev watcher when
/// `III_HARNESS_UI_WATCH` is set. Call after `functions::register_all`.
/// Trigger registration failures are warn-logged, not fatal — injected UI
/// is an accessory, and the SDK replays surviving registrations on
/// reconnect.
pub fn register(iii: &Arc<IIIClient>) {
    let served = Arc::new(Served::new(&ASSETS));

    {
        let served = served.clone();
        iii.register_function(
            CONTENT_FN,
            RegisterFunction::new_async(move |input: UiContentInput| {
                let served = served.clone();
                async move { served.content_for(&input.path).await }
            })
            .description(
                "Serve the harness worker's injected console UI assets (content function \
                 for its console:script / console:style triggers).",
            )
            .metadata(serde_json::json!({ "internal": true })),
        );
    }

    let mut watched = Vec::new();
    for spec in &ASSETS {
        match register_asset_trigger(iii, spec.kind, spec.path) {
            Ok(handle) => {
                tracing::info!(path = spec.path, "registered console ui asset");
                watched.push(WatchedAsset {
                    path: spec.path,
                    kind: spec.kind,
                    file: spec.file,
                    handle,
                    prev: spec.content.to_string(),
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
            spawn_watcher(iii.clone(), served, watched, dist);
        }
    }
}

/// Serve the assets from in-memory cells so the dev watcher can swap the
/// bytes without re-registering the function.
struct Served {
    content: RwLock<HashMap<&'static str, ServedAsset>>,
}

struct ServedAsset {
    content: String,
    content_type: &'static str,
}

impl Served {
    fn new(assets: &[AssetSpec]) -> Self {
        Self {
            content: RwLock::new(
                assets
                    .iter()
                    .map(|a| {
                        (
                            a.path,
                            ServedAsset {
                                content: a.content.to_string(),
                                content_type: a.kind.content_type(),
                            },
                        )
                    })
                    .collect(),
            ),
        }
    }

    async fn content_for(&self, path: &str) -> Result<UiContentResult, Error> {
        let guard = self.content.read().await;
        let asset = guard.get(path).ok_or_else(|| {
            Error::Handler(format!(
                "UNKNOWN_UI_ASSET: '{path}' is not one of this worker's ui assets \
                 (expected one of: {PAGE_PATH}, {STYLES_PATH})"
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
    kind: AssetKind,
    path: &str,
) -> Result<iii_sdk::trigger::Trigger, Error> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: kind.trigger_type().to_string(),
        function_id: CONTENT_FN.to_string(),
        config: serde_json::json!({ "path": path }),
        metadata: None,
    })
}

fn watch_target() -> Option<PathBuf> {
    parse_watch_target(&std::env::var(WATCH_ENV).ok()?)
}

fn parse_watch_target(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw == "0" || raw.eq_ignore_ascii_case("false") {
        return None;
    }
    if raw == "1" || raw.eq_ignore_ascii_case("true") {
        return Some(PathBuf::from(WATCH_DEFAULT_DIR));
    }
    Some(PathBuf::from(raw))
}

struct WatchedAsset {
    path: &'static str,
    kind: AssetKind,
    file: &'static str,
    handle: iii_sdk::trigger::Trigger,
    prev: String,
}

/// Dev-loop hot reload: poll the built files; on change, swap the served
/// bytes, register a fresh trigger for the same path (the console supersedes
/// + re-fetches + pushes to every tab), THEN unregister the previous handle.
fn spawn_watcher(
    iii: Arc<IIIClient>,
    served: Arc<Served>,
    mut watched: Vec<WatchedAsset>,
    dist: PathBuf,
) {
    tokio::spawn(async move {
        tracing::info!(dir = %dist.display(), "ui watch enabled — hot reload on rebuild");
        loop {
            tokio::time::sleep(WATCH_POLL).await;
            for asset in watched.iter_mut() {
                let file = dist.join(asset.file);
                let Ok(next) = tokio::fs::read_to_string(&file).await else {
                    continue;
                };
                if next == asset.prev {
                    continue;
                }
                served.swap(asset.path, next.clone()).await;
                asset.prev = next;
                match register_asset_trigger(&iii, asset.kind, asset.path) {
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

    #[test]
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=harness]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="harness"]"#)
                || STYLES_CSS.contains("[data-iii-ui=harness]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }

    #[tokio::test]
    async fn serves_registered_assets_with_content_types() {
        let served = Served::new(&ASSETS);
        let page = served.content_for(PAGE_PATH).await.unwrap();
        assert!(page.content_type.starts_with("text/javascript"));
        let styles = served.content_for(STYLES_PATH).await.unwrap();
        assert!(styles.content_type.starts_with("text/css"));
    }

    #[tokio::test]
    async fn unknown_path_errors_and_names_the_known_paths() {
        let err = Served::new(&ASSETS)
            .content_for("harness/nope.js")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("UNKNOWN_UI_ASSET"));
        assert!(msg.contains(PAGE_PATH));
        assert!(msg.contains(STYLES_PATH));
    }

    #[test]
    fn parse_watch_target_conventions() {
        assert_eq!(parse_watch_target(""), None);
        assert_eq!(parse_watch_target("0"), None);
        assert_eq!(parse_watch_target("false"), None);
        assert_eq!(parse_watch_target("FALSE"), None);
        assert_eq!(parse_watch_target("1"), Some(PathBuf::from("ui/dist")));
        assert_eq!(parse_watch_target("true"), Some(PathBuf::from("ui/dist")));
        assert_eq!(
            parse_watch_target("build/out"),
            Some(PathBuf::from("build/out"))
        );
    }
}
