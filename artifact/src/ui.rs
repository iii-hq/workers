//! Injectable console UI for the artifact worker.
//!
//! Ships two assets into any running console:
//!
//! - `artifact/page.js` (script) — the Artifacts page (list, sandboxed
//!   preview, Share…, live links with Revoke), the chat card for
//!   `artifact::create`/`update`/`get`/`share`, and the configuration form.
//! - `artifact/styles.css` (style) — every rule scoped under
//!   `[data-iii-ui="artifact"]`.
//!
//! The assets are compiled from `ui/` by esbuild and embedded at compile
//! time. For the dev loop set `III_ARTIFACT_UI_WATCH=1` (or a directory):
//! the worker re-registers a changed asset and every open tab hot-swaps it.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "artifact/page.js";
pub const STYLES_PATH: &str = "artifact/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("artifact")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the console UI. Call after `functions::register_all`.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        // The builder panics on any path or kind the console would reject.
        let _ = console_ui();
    }

    #[test]
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="artifact"]"#)
                || STYLES_CSS.contains("[data-iii-ui=artifact]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
