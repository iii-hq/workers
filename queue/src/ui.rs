//! The queue worker's injected console UI — the Queues page (topics, stats,
//! publish, dead letters with redrive/discard) shipped over the console's
//! `console:script` / `console:style` trigger types.
//!
//! Registration machinery comes from the shared `iii-console-ui` crate
//! (workers/crates/console-ui); this module only names the assets and embeds
//! the bytes built from `ui/` (see `build.rs`). Dev loop:
//! `cd ui && pnpm watch` + `III_QUEUE_UI_WATCH=1`.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "queue/page.js";
pub const STYLES_PATH: &str = "queue/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn queue_ui() -> ConsoleUi {
    ConsoleUi::new("queue")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the queue worker's console assets. Ordering never matters: if
/// the console is not up yet the engine parks the registration and delivers
/// it when the console arrives.
pub fn register(iii: &Arc<IIIClient>) {
    queue_ui().register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builder_accepts_the_assets() {
        // The builder panics on any path/kind the console would reject.
        let _ = queue_ui();
    }

    #[test]
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    /// The page is useless if its id drifts from the route deep links use
    /// (`#/ext/queues`), or if it loses the queue functions it drives.
    #[test]
    fn embedded_page_registers_the_queues_page() {
        assert!(PAGE_JS.contains("queues"));
        for fn_id in [
            "engine::queue::list_topics",
            "engine::queue::dlq_messages",
            "iii::queue::redrive",
            "iii::durable::publish",
        ] {
            assert!(
                PAGE_JS.contains(fn_id),
                "built page.js no longer calls `{fn_id}`"
            );
        }
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=queue]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="queue"]"#)
                || STYLES_CSS.contains("[data-iii-ui=queue]"),
            "built styles.css must be scoped under the queue worker's data-iii-ui attribute"
        );
    }
}
