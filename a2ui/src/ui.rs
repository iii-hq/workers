//! Injectable A2UI Console page and function-result renderer.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "a2ui/page.js";
pub const STYLES_PATH: &str = "a2ui/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("a2ui")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSET_CAP_BYTES: usize = 8 * 1024 * 1024;

    #[test]
    fn ui_assets_are_valid_scoped_and_bounded() {
        let _ = console_ui();
        assert!(PAGE_JS.contains("export"));
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui=\"a2ui\"]"#)
                || STYLES_CSS.contains("[data-iii-ui=a2ui]")
        );
        assert!(STYLES_CSS.contains("container-type: inline-size"));
        assert!(STYLES_CSS.contains("@container a2ui-page"));
        assert!(STYLES_CSS.contains(".a2ui-mobile-switcher"));
        assert!(PAGE_JS.len() < ASSET_CAP_BYTES);
        assert!(STYLES_CSS.len() < ASSET_CAP_BYTES);
    }
}
