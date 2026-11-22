//! Injectable console UI for the mcp-client worker (authoring SOP:
//! workers/docs/sops/injectable-console-ui.md).
//!
//! Ships two assets into any running console: `mcp-client/page.js` (the
//! MCP client page) and `mcp-client/styles.css` (its stylesheet, every rule
//! scoped under `[data-iii-ui="mcp-client"]`). The registration machinery
//! lives in the shared `iii-console-ui` crate; this module only names the
//! assets and embeds their bytes, built from `ui/` by `build.rs`.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "mcp-client/page.js";
pub const STYLES_PATH: &str = "mcp-client/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("mcp-client")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the mcp-client worker's console UI. Call after
/// `functions::register_all`.
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
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=mcp-client]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="mcp-client"]"#)
                || STYLES_CSS.contains("[data-iii-ui=mcp-client]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
