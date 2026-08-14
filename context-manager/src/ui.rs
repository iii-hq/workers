//! Injectable console configuration form for the context-manager worker.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "context-manager/page.js";
pub const STYLES_PATH: &str = "context-manager/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("context-manager")
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
    fn embedded_page_registers_the_config_form() {
        assert!(PAGE_JS.contains("configForms"));
    }

    #[test]
    fn embedded_form_covers_every_worker_setting() {
        for field in [
            "reserved_tokens_cap",
            "reserved_pct",
            "tail_turns",
            "protect_recent_tokens",
            "min_free_tokens",
            "max_output_chars",
            "max_result_tokens",
            "lease_ttl_secs",
            "allow_fallback_limits",
            "summarizer_timeout_ms",
            "lease_dir",
        ] {
            assert!(
                PAGE_JS.contains(field),
                "missing configuration field {field}"
            );
        }
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="context-manager"]"#)
                || STYLES_CSS.contains("[data-iii-ui=context-manager]")
        );
    }
}
