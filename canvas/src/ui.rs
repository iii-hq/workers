//! Injectable console UI for the canvas worker.
//!
//! Ships four assets into any running console:
//!
//! - `canvas/page.js` (`console:script`) — the canvas page (list, edit, draw)
//!   plus the chat renderer for `canvas::*` calls.
//! - `canvas/styles.css` (`console:style`) — the stylesheet, every rule scoped
//!   under `[data-iii-ui="canvas"]`.
//! - `canvas/mermaid.js` (`console:script`) — the bundled mermaid renderer,
//!   lazily `import()`ed by page.js via `ui/src/lib/loaders.ts`.
//! - `canvas/freeform.js` (`console:script`) — the bundled excalidraw
//!   whiteboard + mermaid→excalidraw converter, same lazy-load path.
//!
//! The vendor bundles are `.js`, not `.mjs`: the console only accepts script
//! paths ending in `.js` (the `iii-console-ui` builder panics on anything
//! else). And because the console eagerly `import()`s every `console:script`
//! asset and calls its default export as `setup(host)`, both vendor entries
//! default-export a no-op setup and carry their real payload as named exports
//! (see `ui/mermaid-entry.ts` / `ui/freeform-entry.ts`).
//!
//! The registration machinery (content function `canvas::ui-content`, one
//! Message-path trigger per asset, the `III_CANVAS_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate; this module only names
//! the assets and embeds their bytes.
//!
//! The assets are compiled from `ui/` by esbuild (react and
//! `@iii-dev/console-ui` external for page.js, so they resolve through the
//! console's import map at runtime) and embedded at compile time, so the
//! worker stays one self-contained binary.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "canvas/page.js";
pub const STYLES_PATH: &str = "canvas/styles.css";
pub const MERMAID_PATH: &str = "canvas/mermaid.js";
pub const FREEFORM_PATH: &str = "canvas/freeform.js";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));
const MERMAID_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/mermaid.js"));
const FREEFORM_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/freeform.js"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("canvas")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
        .script(MERMAID_PATH, MERMAID_JS)
        .script(FREEFORM_PATH, FREEFORM_JS)
}

/// Register the canvas worker's console UI. Call after the functions it
/// drives.
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
        // The builder panics on any path or kind the console would reject.
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
        // esbuild prints the attribute selector unquoted ([data-iii-ui=canvas]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="canvas"]"#)
                || STYLES_CSS.contains("[data-iii-ui=canvas]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }

    /// The console eagerly imports every `console:script` asset and calls its
    /// default export as `setup(host)`; a vendor bundle without one logs a
    /// loader error in every tab.
    #[test]
    fn embedded_vendor_bundles_are_esm_with_a_default_export() {
        for (path, content) in [(MERMAID_PATH, MERMAID_JS), (FREEFORM_PATH, FREEFORM_JS)] {
            assert!(content.contains("export"), "built {path} looks wrong");
            assert!(
                content.contains("default"),
                "built {path} must default-export a no-op setup for the console loader"
            );
        }
    }

    /// The console rejects any asset over 8 MiB; the vendor bundles are the
    /// ones that can silently grow past it on a dependency bump.
    #[test]
    fn every_embedded_asset_stays_under_the_console_cap() {
        for (path, content) in [
            (PAGE_PATH, PAGE_JS),
            (STYLES_PATH, STYLES_CSS),
            (MERMAID_PATH, MERMAID_JS),
            (FREEFORM_PATH, FREEFORM_JS),
        ] {
            assert!(
                content.len() < ASSET_CAP_BYTES,
                "{path} is {} bytes — past the console's {ASSET_CAP_BYTES}-byte cap",
                content.len()
            );
        }
    }
}
