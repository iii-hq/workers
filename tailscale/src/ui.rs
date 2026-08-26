use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "tailscale/page.js";
pub const STYLES_PATH: &str = "tailscale/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("tailscale")
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
    fn assets_are_built_and_scoped() {
        let _ = console_ui();
        assert!(PAGE_JS.contains("export"));
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="tailscale"]"#)
                || STYLES_CSS.contains("[data-iii-ui=tailscale]")
        );
    }
}
