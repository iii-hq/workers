//! Injectable Console UI for the compose supervisor.
//!
//! `build.rs` compiles the React page and stylesheet before this module embeds
//! them. The shared `iii-console-ui` builder owns the content function,
//! Message-path asset triggers, validation, and `III_COMPOSE_UI_UI_WATCH`
//! hot-reload loop.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "compose-ui/page.js";
pub const STYLES_PATH: &str = "compose-ui/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("compose-ui")
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
            STYLES_CSS.contains(r#"[data-iii-ui="compose-ui"]"#)
                || STYLES_CSS.contains("[data-iii-ui=compose-ui]"),
            "built styles.css must be scoped under compose-ui's data-iii-ui attribute"
        );
    }
}
