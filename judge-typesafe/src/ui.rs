//! Embed and register the JEV console configuration form.
use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;
use std::sync::Arc;

pub const PAGE_PATH: &str = "judge-typesafe/page.js";
pub const STYLES_PATH: &str = "judge-typesafe/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("judge-typesafe")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register after the worker's public functions.
pub fn register(iii: &Arc<IIIClient>) {
    iii_console_ui::register_configuration_identity(
        iii,
        "judge-typesafe",
        crate::configuration::config_id(),
    );
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
    fn embedded_page_is_nonempty_esm() {
        assert!(
            PAGE_JS.contains("export"),
            "page.js must be a nonempty ES module"
        );
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="judge-typesafe"]"#)
                || STYLES_CSS.contains("[data-iii-ui=judge-typesafe]"),
            "styles.css must be scoped to the JEV worker"
        );
    }
}
