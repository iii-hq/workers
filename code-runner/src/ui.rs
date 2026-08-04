//! Injectable console UI for the code-runner worker
//! (iii/tech-specs/2026-07-17-injectable-ui; authoring SOP:
//! workers/docs/sops/injectable-console-ui.md).
//!
//! Ships two assets into any running console:
//!
//! - `code-runner/page.js` (`console:script`) — the function-trigger
//!   renderers its `setup(host)` registers, so `code-runner::eval`,
//!   `register_function` and `teardown` render as purpose-built cards in chat
//!   and traces instead of raw JSON.
//! - `code-runner/styles.css` (`console:style`) — the stylesheet, every rule
//!   scoped under `[data-iii-ui="code-runner"]`; the console mounts it as a
//!   `<link>` and link-swaps it on change, styles-before-scripts on boot.
//!
//! The registration machinery (content function `code-runner::ui-content`,
//! one Message-path trigger per asset, `III_CODE_RUNNER_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate (path-linked from
//! `workers/crates/console-ui`); this module only names the assets and embeds
//! their bytes.
//!
//! The assets are compiled from `ui/` by esbuild (react + @iii-dev/console-ui
//! external — they resolve through the console's import map at runtime) and
//! embedded at compile time so the worker stays one self-contained binary.
//! For the dev loop, set `III_CODE_RUNNER_UI_WATCH` to the build output
//! directory (or `1` for `ui/dist`): the worker polls both files and
//! re-registers a changed asset's trigger — every open console tab hot-swaps
//! it.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "code-runner/page.js";
pub const STYLES_PATH: &str = "code-runner/styles.css";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("code-runner")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
}

/// Register the code-runner worker's console UI. Call after
/// `functions::register_all`.
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
    fn embedded_page_is_nonempty_esm() {
        assert!(PAGE_JS.contains("export"), "built page.js looks wrong");
    }

    /// One renderer per user-facing op, all reachable from `setup(host)`.
    /// Dropping one from `ui/src/function-trigger-message/index.tsx` silently
    /// falls that op back to the console's raw-JSON card; this catches it.
    #[test]
    fn every_rendered_op_is_wired_in() {
        for op in ["eval", "register_function", "teardown"] {
            assert!(
                PAGE_JS.contains(&format!("code-runner::{op}")),
                "no renderer for code-runner::{op} in the built page.js"
            );
        }
    }

    /// `inject-guidance` is a harness-internal `pre_generate` hook, not a call
    /// anyone makes; rendering it would put a card in front of a mechanism.
    #[test]
    fn the_guidance_hook_is_never_rendered() {
        assert!(
            !PAGE_JS.contains(crate::functions::inject_guidance::GUIDANCE_HOOK_ID),
            "the harness-internal guidance hook must not have a renderer"
        );
    }

    /// A bundled React copy surfaces at runtime as a cryptic "Invalid hook
    /// call" — the shared specifiers have to stay bare imports resolved by the
    /// console's import map.
    #[test]
    fn react_stays_external() {
        for internal in ["__SECRET_INTERNALS", "ReactCurrentDispatcher"] {
            assert!(
                !PAGE_JS.contains(internal),
                "react appears to be bundled into page.js ({internal} found)"
            );
        }
        // Every card uses hooks, so react must be in there — as a bare import
        // the console's import map resolves, never as bundled source.
        assert!(
            PAGE_JS.contains(r#"from "react"#),
            "react should be imported, not bundled"
        );
    }

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted ([data-iii-ui=code-runner]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="code-runner"]"#)
                || STYLES_CSS.contains("[data-iii-ui=code-runner]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
