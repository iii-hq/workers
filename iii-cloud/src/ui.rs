//! Injectable console UI for the iii-cloud worker: the `iii cloud` page and its
//! stylesheet, built from `ui/` by esbuild and embedded at compile time so the
//! worker stays one binary. The registration machinery (content function
//! `iii-cloud::ui-content`, the `ade::ui::asset` observers, the
//! `III_III_CLOUD_UI_WATCH` hot-reload watcher) lives in `iii-console-ui`.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "iii-cloud/page.js";
pub const STYLES_PATH: &str = "iii-cloud/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("iii-cloud")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the console UI. Call after the `iii-cloud::*` functions.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        // The builder panics on any path/kind the ADE worker would reject.
        let _ = console_ui();
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="iii-cloud"]"#)
                || STYLES_CSS.contains("[data-iii-ui=iii-cloud]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
