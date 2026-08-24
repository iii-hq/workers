//! Injectable Console UI for security scan runs and reports.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "security-scan/page.js";
pub const STYLES_PATH: &str = "security-scan/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("security-scan")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the scanner page after its public functions are available.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_embedded_assets() {
        let _ = console_ui();
    }

    #[test]
    fn embedded_page_uses_public_scanner_contracts() {
        assert!(PAGE_JS.contains("export"), "built page.js is not ESM");
        assert!(PAGE_JS.contains("security-scan::list"));
        assert!(PAGE_JS.contains("security-scan::read"));
        assert!(PAGE_JS.contains("security-scan::request"));
        assert!(PAGE_JS.contains("security-scan::cancel"));
        assert!(PAGE_JS.contains("start scan"));
        assert!(PAGE_JS.contains("stop scan"));
        assert!(PAGE_JS.contains("entire repo analysis"));
        assert!(PAGE_JS.contains("HEAD ->"));
        assert!(PAGE_JS.contains("selectConversation"));
        assert!(PAGE_JS.contains("composerModel"));
        assert!(PAGE_JS.contains("session::get"));
        assert!(PAGE_JS.contains("security-scan:runs"));
        assert!(!PAGE_JS.contains("state::get"));
        assert!(!PAGE_JS.contains("state::list"));
    }

    #[test]
    fn embedded_styles_are_worker_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="security-scan"]"#)
                || STYLES_CSS.contains("[data-iii-ui=security-scan]"),
            "built styles.css must use the security-scan host scope"
        );
    }
}
