//! Injectable console UI (authoring contract: `ade/skills/injectable-ui.md`).
//!
//! - `ios-simulator/page.js` (`console:script`) — the Simulators page (device
//!   rail, live iPhone with multi-touch, buttons and media), the
//!   configuration form, and the chat renderer for `ios-simulator::*` calls.
//! - `ios-simulator/styles.css` (`console:style`) — every rule scoped under
//!   `[data-iii-ui="ios-simulator"]`.
//!
//! Built from `ui/` by `build.rs` and embedded, so the worker stays one
//! binary. `III_IOS_SIMULATOR_UI_WATCH=1` hot-reloads `ui/dist` in development.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "ios-simulator/page.js";
pub const STYLES_PATH: &str = "ios-simulator/styles.css";

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("ios-simulator")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the console UI. Call after the `ios-simulator::*` functions.
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
    fn embedded_page_is_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="ios-simulator"]"#)
                || STYLES_CSS.contains("[data-iii-ui=ios-simulator]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
