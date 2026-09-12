//! Injectable console UI for the kanban worker (authoring SOP:
//! `workers/docs/sops/injectable-console-ui.md`).
//!
//! Two assets, built from `ui/` by esbuild and embedded at compile time:
//! `kanban/page.js` (`console:script`) registers the board page, the ticket
//! page, the configuration form and the function/trigger renderers;
//! `kanban/styles.css` (`console:style`) is the stylesheet, every rule scoped
//! under `[data-iii-ui="kanban"]`. Set `III_KANBAN_UI_WATCH=1` for hot reload
//! from `ui/dist` during development.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "kanban/page.js";
pub const STYLES_PATH: &str = "kanban/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("kanban")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the console UI. Call after the worker's functions are registered.
pub fn register(iii: &Arc<IIIClient>) {
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
    fn embedded_page_registers_every_kanban_surface() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
        for needle in [
            "kanban-board",
            "kanban-ticket",
            "configForms",
            "functionTriggers",
            "triggerRenderers",
            "kanban/page.js#ticket",
            "kanban/page.js#trigger-activity",
        ] {
            assert!(
                PAGE_JS.contains(needle),
                "built page.js must contain {needle}"
            );
        }
    }

    #[test]
    fn embedded_page_does_not_bundle_react() {
        assert!(
            !PAGE_JS.contains("ReactCurrentDispatcher")
                && !PAGE_JS.contains("__SECRET_INTERNALS_DO_NOT_USE"),
            "built page.js must use the console's React runtime"
        );
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="kanban"]"#)
                || STYLES_CSS.contains("[data-iii-ui=kanban]"),
            "built styles.css must be scoped under the kanban worker"
        );
    }
}
