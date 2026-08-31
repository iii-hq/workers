//! Injectable console UI for the cron worker's schedule page, configuration,
//! and source-specific trigger activity.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "cron/page.js";
pub const STYLES_PATH: &str = "cron/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("cron")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

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
    fn embedded_page_registers_all_cron_surfaces() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
        assert!(
            PAGE_JS.contains("configForms"),
            "built page.js must register the cron configuration form"
        );
        assert!(
            PAGE_JS.contains("triggerRenderers"),
            "built page.js must register the trigger activity renderer"
        );
        assert!(
            PAGE_JS.contains("cron/page.js#trigger-activity"),
            "built page.js must retain the renderer identity"
        );
    }

    #[test]
    fn embedded_page_does_not_bundle_react() {
        assert!(
            !PAGE_JS.contains("ReactCurrentDispatcher")
                && !PAGE_JS.contains("__SECRET_INTERNALS_DO_NOT_USE"),
            "built page.js must use the Console's React runtime"
        );
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="cron"]"#)
                || STYLES_CSS.contains("[data-iii-ui=cron]"),
            "built styles.css must be scoped under the cron worker"
        );
    }
}
