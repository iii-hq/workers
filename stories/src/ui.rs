//! Injectable console UI for the stories worker plus the `stories::ui-file`
//! content function the console's `/ui-files/stories/...` route pulls
//! built story documents through.
//!
//! Two assets, built from `ui/` by esbuild and embedded at compile time:
//! `stories/page.js` (`console:script`) registers the explorer/compare page
//! and the configuration form; `stories/styles.css` (`console:style`) is the
//! stylesheet, every rule scoped under `[data-iii-ui="stories"]`. Set
//! `III_STORIES_UI_WATCH=1` for hot reload from `ui/dist`.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use iii_console_ui::ConsoleUi;
use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::builder::Ctx;
use crate::store::safe_served_path;

pub const PAGE_PATH: &str = "stories/page.js";
pub const STYLES_PATH: &str = "stories/styles.css";
pub const UI_FILE_FN_ID: &str = "stories::ui-file";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("stories")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UiFileInput {
    /// `<workspace>/<line key>/<served path>`.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UiFileOutput {
    pub content_base64: String,
    pub content_type: String,
    /// `immutable` for hashed assets, `no-cache` for documents.
    pub cache: String,
}

/// `<workspace>/<line>/<served path>` → the served path's bytes.
pub fn split_path(path: &str) -> Option<(&str, &str, &str)> {
    let path = path.trim_matches('/');
    let mut parts = path.splitn(3, '/');
    let workspace = parts.next()?;
    let line = parts.next()?;
    let rest = parts.next()?;
    if ![workspace, line, rest].iter().all(|s| safe_served_path(s)) {
        return None;
    }
    Some((workspace, line, rest))
}

pub fn register(ctx: &Arc<Ctx>) {
    let captured = ctx.clone();
    ctx.iii.register_function(
        UI_FILE_FN_ID,
        RegisterFunction::new_async(move |input: UiFileInput| {
            let ctx = captured.clone();
            async move {
                let (workspace, line, rest) = split_path(&input.path)
                    .ok_or_else(|| Error::Handler(format!("bad ui-file path: {}", input.path)))?;
                let (bytes, content_type) =
                    ctx.store.serve(workspace, line, rest).ok_or_else(|| {
                        Error::Handler(format!(
                            "no such file in line {line} of {workspace}: {rest}"
                        ))
                    })?;
                let cache = if rest.contains("/assets/") {
                    "immutable"
                } else {
                    "no-cache"
                };
                Ok::<_, Error>(UiFileOutput {
                    content_base64: STANDARD.encode(bytes),
                    content_type,
                    cache: cache.to_string(),
                })
            }
        })
        .description(
            "Internal: serves a built story document or asset to the console's /ui-files route.",
        ),
    );
    console_ui().register(&ctx.iii);
}

pub fn register_assets_only(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        let _ = console_ui();
    }

    #[test]
    fn embedded_page_registers_the_page() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
        for needle in ["stories-explorer", "configForms"] {
            assert!(
                PAGE_JS.contains(needle),
                "built page.js must contain {needle}"
            );
        }
        assert!(
            STYLES_CSS.contains("data-iii-ui=stories")
                || STYLES_CSS.contains("data-iii-ui=\"stories\"")
        );
    }

    #[test]
    fn ui_file_paths_split_into_workspace_line_and_rest() {
        assert_eq!(
            split_path("ws/worktree/app/node_modules/.stories/x.html"),
            Some(("ws", "worktree", "app/node_modules/.stories/x.html"))
        );
        assert_eq!(split_path("ws/worktree"), None);
        assert_eq!(split_path("../worktree/index.html"), None);
        assert_eq!(split_path("ws/../index.html"), None);
        assert_eq!(split_path("ws/worktree/../index.html"), None);
        assert_eq!(
            split_path("/ws/abc/assets/a.js"),
            Some(("ws", "abc", "assets/a.js"))
        );
    }
}
