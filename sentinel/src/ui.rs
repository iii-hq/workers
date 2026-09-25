//! The injected console page.
//!
//! The page is an asset of this worker rather than a screen of the console:
//! it ships with the binary, it appears when the worker is up and disappears
//! when it is not, and it calls this worker's own functions and no others —
//! `engine::traces::*` is no more the console's to read than it is an
//! investigation's.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "sentinel/page.js";
pub const STYLES_PATH: &str = "sentinel/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("sentinel")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the page once the functions it calls are registered.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_accepts_the_embedded_assets() {
        let _ = console_ui();
    }

    #[test]
    fn the_page_calls_this_workers_functions_and_not_the_engines() {
        assert!(PAGE_JS.contains("export"), "the built page is not ESM");
        for function_id in [
            "sentinel::groups::list",
            "sentinel::groups::get",
            "sentinel::groups::resolve",
            "sentinel::groups::ignore",
            "sentinel::evidence::get",
            "sentinel::diagnoses::list",
            "sentinel::investigate",
            "sentinel::investigations::cancel",
            "sentinel::diagnosis::record",
            "sentinel::group-changed",
            "sentinel::investigation-changed",
        ] {
            assert!(
                PAGE_JS.contains(function_id),
                "{function_id} is not called by the page"
            );
        }
        // What the page shows is the frozen bundle. Reading the live engine
        // is the investigation's business, through the redacting proxies.
        assert!(
            !PAGE_JS.contains("engine::traces::"),
            "the page must not read the engine's telemetry directly"
        );
        assert!(
            !PAGE_JS.contains("database::"),
            "the page never touches the store"
        );
        // One deliberate exception to "this worker's functions": the model
        // picker is fed by the router's catalog and re-read when the router
        // says it changed, the same way `security-scan` feeds its own.
        for function_id in ["router::models::list", "router::models::changed"] {
            assert!(
                PAGE_JS.contains(function_id),
                "the model picker needs {function_id}"
            );
        }
    }

    #[test]
    fn the_page_opens_the_investigation_beside_itself() {
        assert!(
            PAGE_JS.contains("selectConversation"),
            "Investigate has to put the session in front of the user"
        );
    }

    #[test]
    fn the_styles_are_scoped_to_this_worker() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="sentinel"]"#)
                || STYLES_CSS.contains("[data-iii-ui=sentinel]"),
            "every selector must be scoped to the sentinel host element"
        );
    }
}
