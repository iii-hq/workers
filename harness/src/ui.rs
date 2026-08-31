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
//! The registration machinery (content function `harness::ui-content`, one
//! Message-path trigger per asset, `III_HARNESS_UI_WATCH` hot-reload watcher)
//! lives in the shared `iii-console-ui` crate (path-linked from
//! `workers/crates/console-ui`); this module only names the assets and
//! embeds their bytes.
//!
//! The assets are compiled from `ui/` by esbuild (react + @iii-dev/console-ui
//! external — they resolve through the console's import map at runtime) and
//! embedded at compile time so the worker stays one self-contained binary.
//! For the dev loop, set `III_HARNESS_UI_WATCH` to the build output
//! directory (or `1` for `ui/dist`): the worker polls both files and
//! re-registers a changed asset's trigger — every open console tab
//! hot-swaps it.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "harness/page.js";
pub const STYLES_PATH: &str = "harness/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("harness")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the harness's console UI. Call after `functions::register_all`.
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
        // esbuild prints the attribute selector unquoted ([data-iii-ui=harness]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="harness"]"#)
                || STYLES_CSS.contains("[data-iii-ui=harness]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
