//! Injectable console UI for viewable `web::fetch` results.
//!
//! The worker ships its renderer and scoped stylesheet as self-contained
//! assets. This keeps rich chat behavior with the function owner: adding a
//! new web result type does not require a console SPA release.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "web/page.js";
pub const STYLES_PATH: &str = "web/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("web")
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
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="web"]"#)
                || STYLES_CSS.contains("[data-iii-ui=web]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
