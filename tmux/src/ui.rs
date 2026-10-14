use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "tmux/page.js";
pub const STYLES_PATH: &str = "tmux/styles.css";
/// xterm ships as a `console:module` the page imports when the terminal mounts.
pub const XTERM_PATH: &str = "tmux/xterm.js";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));
const XTERM_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/xterm.js"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("tmux")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
        .module(XTERM_PATH, XTERM_JS)
}

/// Call once, after the worker's functions are registered.
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
        assert!(PAGE_JS.contains("export"));
    }

    #[test]
    fn embedded_styles_are_scoped() {
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="tmux"]"#)
                || STYLES_CSS.contains("[data-iii-ui=tmux]")
        );
    }

    #[test]
    fn xterm_ships_as_its_own_module() {
        assert!(XTERM_JS.contains("xterm-helper-textarea"));
        assert!(
            !PAGE_JS.contains("xterm-helper-textarea"),
            "the page must not bundle xterm"
        );
    }
}
