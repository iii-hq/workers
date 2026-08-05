//! The console worker's *own* injected UI — shipped through the same
//! `console:script` / `console:style` trigger types it hosts (the engine
//! routes the registration straight back to this worker).
//!
//! Two contributions:
//!
//! - a custom configuration form for the `console` entry
//!   (`host.configForms`), replacing the schema-generated JSON editor with
//!   the injectable-UI toggle board — one bordered card per worker (title +
//!   description + switch) flipping `injectableUi.disabledWorkers`, which
//!   [`crate::configuration::start_injectable_ui_sync`] applies live;
//! - the engine-catalogue pages (`host.pages`): functions and triggers,
//!   reading `engine::functions::*` / `engine::triggers::*` /
//!   `engine::registered-triggers::list`. They are engine-level views no
//!   single worker owns, and they ship injected so the console SPA carries
//!   no per-view code.
//!
//! Registration machinery comes from the shared `iii-console-ui` crate
//! (workers/crates/console-ui); this module only names the assets and embeds
//! the bytes built from `ui/` (see `build.rs`). Dev loop:
//! `cd ui && pnpm watch` + `III_CONSOLE_UI_WATCH=1`.
//!
//! The registry refuses to disable the `console` worker itself
//! (`ui_assets::UNDISABLEABLE_WORKER`), so this form cannot lock the
//! operator out of its own toggles.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const CONFIG_FORM_PATH: &str = "console/config-form.js";
pub const CATALOG_PAGE_PATH: &str = "console/catalog-page.js";
pub const STYLES_PATH: &str = "console/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const CONFIG_FORM_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/ui/dist/config-form.js"
));
const CATALOG_PAGE_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/ui/dist/catalog-page.js"
));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("console")
        .script(CONFIG_FORM_PATH, CONFIG_FORM_JS)
        .script(CATALOG_PAGE_PATH, CATALOG_PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the console's own UI assets. Call only when `injectable_ui` is
/// on (the trigger types must exist), after `ui_assets::start`.
pub fn register(iii: &Arc<IIIClient>) {
    console_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        // The builder panics on any path/kind the console would reject.
        let _ = console_ui();
    }

    #[test]
    fn embedded_form_is_nonempty_esm() {
        assert!(
            CONFIG_FORM_JS.contains("export"),
            "built config-form.js looks wrong"
        );
    }

    #[test]
    fn embedded_catalog_page_is_nonempty_esm() {
        assert!(
            CATALOG_PAGE_JS.contains("export"),
            "built catalog-page.js looks wrong"
        );
    }

    /// The pages are useless if their ids drift from the routes the nav and
    /// deep links use (`#/ext/functions`, `#/ext/triggers`).
    #[test]
    fn embedded_catalog_page_registers_both_pages() {
        assert!(CATALOG_PAGE_JS.contains("functions"));
        assert!(CATALOG_PAGE_JS.contains("triggers"));
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=console]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="console"]"#)
                || STYLES_CSS.contains("[data-iii-ui=console]"),
            "built styles.css must be scoped under the console's data-iii-ui attribute"
        );
    }
}
