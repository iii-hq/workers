//! Injectable console UI for the voice worker.
//!
//! Ships two assets into any running console:
//!
//! - `voice/page.js` (`console:script`) — the mic chip in every chat header,
//!   the read-aloud turn summary above the composer, the voice page
//!   (`#/ext/voice`) and the palette commands.
//! - `voice/styles.css` (`console:style`) — the stylesheet, every rule scoped
//!   under `[data-iii-ui="voice"]`.
//!
//! The registration machinery (content function `voice::ui-content`, one
//! Message-path trigger per asset, the `III_VOICE_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate; this module only names
//! the assets and embeds their bytes.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "voice/page.js";
pub const STYLES_PATH: &str = "voice/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("voice")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the voice worker's console UI. Call after the functions it drives.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The console's hard cap on one injected asset.
    const ASSET_CAP_BYTES: usize = 8 * 1024 * 1024;

    #[test]
    fn ui_builder_accepts_the_assets() {
        let _ = console_ui();
    }

    #[test]
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    /// An unscoped rule in injected CSS is unlayered and silently beats the
    /// console's own styles document-wide.
    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="voice"]"#)
                || STYLES_CSS.contains("[data-iii-ui=voice]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }

    #[test]
    fn every_embedded_asset_stays_under_the_console_cap() {
        for (path, content) in [(PAGE_PATH, PAGE_JS), (STYLES_PATH, STYLES_CSS)] {
            assert!(
                content.len() < ASSET_CAP_BYTES,
                "{path} is {} bytes — past the console's {ASSET_CAP_BYTES}-byte cap",
                content.len()
            );
        }
    }
}
