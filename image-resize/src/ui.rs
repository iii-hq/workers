//! Injectable console UI for the image-resize worker.
//!
//! Ships two assets into any running console:
//!
//! - `image-resize/page.js` (a `script` asset): a page that resizes an image
//!   and shows it beside the original.
//! - `image-resize/styles.css` (a `style` asset): the stylesheet, every rule
//!   scoped under `[data-iii-ui="image-resize"]`.
//!
//! The registration machinery (content function `image-resize::ui-content`,
//! one `ade::ui::asset` observer per asset, the `III_IMAGE_RESIZE_UI_WATCH`
//! hot-reload watcher) lives in the shared `iii-console-ui` crate; this module
//! only names the assets and embeds their bytes.
//!
//! The assets are compiled from `ui/` by esbuild (react and
//! `@iii-dev/console-ui` external, so they resolve through the console's import
//! map at runtime) and embedded at compile time, so the worker stays one
//! self-contained binary.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "image-resize/page.js";
pub const STYLES_PATH: &str = "image-resize/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("image-resize")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the image-resize worker's console UI. Call after the functions it drives.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        // The builder panics on any path or kind the ADE worker would reject.
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
        // esbuild prints the attribute selector unquoted ([data-iii-ui=image-resize]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="image-resize"]"#)
                || STYLES_CSS.contains("[data-iii-ui=image-resize]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
