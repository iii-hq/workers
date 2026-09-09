//! Provider-owned configuration UI for the chat model picker.

use std::sync::Arc;

use crate::request::CODEX_COMPAT_VERSION;
use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "provider-openai-codex/page.js";
pub const STYLES_PATH: &str = "provider-openai-codex/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

/// Placeholder in `ui/page.tsx`; the form shows which Codex CLI this worker mirrors.
const COMPAT_VERSION_PLACEHOLDER: &str = "__CODEX_COMPAT_VERSION__";

fn page_js() -> String {
    PAGE_JS.replace(COMPAT_VERSION_PLACEHOLDER, CODEX_COMPAT_VERSION)
}

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("provider-openai-codex")
        .script(PAGE_PATH, page_js())
        .style(STYLES_PATH, STYLES_CSS)
}

pub fn register(iii: &IIIClient) {
    console_ui().register(&Arc::new(iii.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        let _ = console_ui();
    }

    #[test]
    fn embedded_page_registers_the_provider_form() {
        assert!(PAGE_JS.contains("providerConfigForms"));
        assert!(PAGE_JS.contains("codex login"));
    }

    #[test]
    fn embedded_page_shows_the_compat_version() {
        assert!(PAGE_JS.contains(COMPAT_VERSION_PLACEHOLDER));
        let js = page_js();
        assert!(js.contains(CODEX_COMPAT_VERSION));
        assert!(!js.contains(COMPAT_VERSION_PLACEHOLDER));
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(STYLES_CSS.contains("[data-iii-ui=provider-openai-codex]"));
    }
}
