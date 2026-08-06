//! Injectable console UI for the llm-router worker
//! (iii/tech-specs/2026-07-17-injectable-ui; authoring SOP:
//! workers/docs/sops/injectable-console-ui.md).
//!
//! Ships two assets into any running console:
//!
//! - `llm-router/page.js` (`console:script`) — the custom configuration form
//!   for the `llm-router` entry: per-provider credential cards with
//!   plain-text-secret detection on api keys (steers operators to
//!   `${ENV_VAR}` references, which the configuration worker expands on
//!   read), the default-provider picker, stream budgets, and the
//!   routing-heuristics table.
//! - `llm-router/styles.css` (`console:style`) — its stylesheet, every rule
//!   scoped under `[data-iii-ui="llm-router"]`.
//!
//! The registration machinery (content function `llm-router::ui-content`,
//! one Message-path trigger per asset, `III_LLM_ROUTER_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate; this module only
//! names the assets and embeds their bytes.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "llm-router/page.js";
pub const STYLES_PATH: &str = "llm-router/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("llm-router")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the llm-router console UI. Call after the router's functions
/// are registered. Takes the bare client (what `register_worker` hands
/// `main`); the shared crate wants an `Arc` for its spawned tasks.
pub fn register(iii: &IIIClient) {
    console_ui().register(&Arc::new(iii.clone()));
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
    fn embedded_page_registers_the_config_form() {
        // The configuration form is the whole point of this asset; a build
        // that lost it silently reverts the Workers tab to the generic
        // schema editor.
        assert!(
            PAGE_JS.contains("configForms"),
            "built page.js no longer registers the configuration form"
        );
    }

    #[test]
    fn embedded_page_carries_the_env_var_guidance() {
        // The plain-text-secret warning is the UX this UI exists for.
        assert!(
            PAGE_JS.contains("plain-text secret"),
            "built page.js lost the plain-text api-key warning"
        );
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild drops the attribute quotes (llm-router is a valid ident).
        assert!(
            STYLES_CSS.contains("[data-iii-ui=llm-router]"),
            "styles.css must scope every rule under the wrapper attribute"
        );
    }
}
