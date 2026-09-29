//! Injected console UI: the chat card for `provider::openai::image::*`.
//!
//! `image::generate` returns only the saved path (no base64 in the
//! transcript), so the console needs a renderer that fetches the preview
//! from this worker over the tab's bus client. Assets are built from `ui/`
//! by build.rs and embedded here; `ConsoleUi` registers the content function
//! and one Message-path trigger per asset (hot-reloadable, GC'd on disconnect).

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "provider-openai/page.js";
pub const STYLES_PATH: &str = "provider-openai/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("provider-openai")
        .script(PAGE_PATH, PAGE_JS)
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
    fn embedded_page_registers_the_image_renderer() {
        assert!(PAGE_JS.contains("functionTriggers"));
        // The ids are assembled from a prefix in the bundle.
        assert!(PAGE_JS.contains("provider::openai::"));
        assert!(PAGE_JS.contains("image::generate"));
        assert!(PAGE_JS.contains("image::read"));
        assert!(!PAGE_JS.contains("openai-codex"));
        // The shared runtime must stay external: a bundled React fails hooks.
        assert!(PAGE_JS.contains("from\"react\"") || PAGE_JS.contains("from \"react\""));
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(STYLES_CSS.contains("[data-iii-ui=provider-openai]"));
        assert!(!STYLES_CSS.contains("provider-openai-codex"));
    }
}
