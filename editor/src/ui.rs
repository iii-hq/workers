//! Injectable console UI for the editor worker
//! (authoring SOP: `workers/docs/sops/injectable-console-ui.md`).
//!
//! Ships two assets into any running console:
//!
//! - `editor/page.js` (`console:script`) — the editor page (file tree, tabs,
//!   Monaco, diff view, live activity), plus the function-trigger renderers
//!   its `setup(host)` registers so `editor::*` calls read as diffs and file
//!   cards in chat instead of raw JSON.
//! - `editor/styles.css` (`console:style`) — every rule scoped under
//!   `[data-iii-ui="editor"]`.
//!
//! The registration machinery (content function `editor::ui-content`, one
//! Message-path trigger per asset, the `III_EDITOR_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate; this module only names
//! the assets and embeds their bytes.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "editor/page.js";
pub const STYLES_PATH: &str = "editor/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("editor")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the editor worker's console UI. Call after `functions::register_all`.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        // The builder panics on any path/kind the console would reject.
        let _ = console_ui();
    }

    #[test]
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=editor]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="editor"]"#)
                || STYLES_CSS.contains("[data-iii-ui=editor]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
