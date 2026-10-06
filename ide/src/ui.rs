//! Injectable console UI for the shell worker
//! (iii/tech-specs/2026-07-17-injectable-ui; authoring SOP:
//! workers/docs/sops/injectable-console-ui.md).
//!
//! Ships three assets into any running console:
//!
//! - `ide/page.js` (`console:script`) — the shell explorer page
//!   (page `ide`: file tree / git / search sidebar beside the shared
//!   Monaco editor and FileDiff pane), plus the `shell::*`
//!   function-trigger renderer its `setup(host)` registers (moved out of
//!   the console SPA, the iii-directory precedent).
//! - `ide/styles.css` (`console:style`) — the stylesheet, every rule
//!   scoped under `[data-iii-ui="ide"]`.
//! - `ide/xterm.js` (`console:module`) — the xterm terminal emulator, which
//!   page.js imports with `host.importModule` only when a terminal opens.
//!
//! The registration machinery (content function `shell::ui-content`, one
//! Message-path trigger per asset, `III_SHELL_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate (path-linked from
//! `workers/crates/console-ui`); this module only names the assets and
//! embeds their bytes. The page's per-pane state (browsed folder, open
//! tabs, terminal layout) is served by `ui_state.rs` from the worker's
//! data directory — it used to live in a `shell-ui` configuration entry,
//! which the worker no longer registers.
//!
//! The assets are compiled from `ui/` by esbuild (react +
//! @iii-dev/console-ui external — they resolve through the console's
//! import map at runtime) and embedded at compile time so the worker
//! stays one self-contained binary. For the dev loop, set
//! `III_SHELL_UI_WATCH` to the build output directory (or `1` for
//! `ui/dist`): the worker polls every built asset and re-registers a changed
//! asset's trigger — every open console tab hot-swaps it (xterm.js alone
//! reaches a tab's terminals once page.js reloads: page.js keeps the module
//! it imported).

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "ide/page.js";
pub const STYLES_PATH: &str = "ide/styles.css";
pub const XTERM_PATH: &str = "ide/xterm.js";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));
const XTERM_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/xterm.js"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("ide")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
        .module(XTERM_PATH, XTERM_JS)
}

/// Register the shell worker's console UI. Call after the function
/// surface is registered.
///
/// The `iii-console-ui` crate wants an `Arc<IIIClient>` (its watcher task
/// clones it); shell passes the client handle around by value everywhere
/// (`IIIClient` is `Clone` — a cheap handle over the shared connection),
/// so the Arc is built here rather than rippling through main.
pub fn register(iii: &IIIClient) {
    console_ui().register(&std::sync::Arc::new(iii.clone()));
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

    #[test]
    fn embedded_styles_are_scoped() {
        // esbuild prints the attribute selector unquoted
        // ([data-iii-ui=ide]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="ide"]"#)
                || STYLES_CSS.contains("[data-iii-ui=ide]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }

    /// The textarea class xterm creates for keyboard input: survives
    /// minification and appears nowhere else in the bundle.
    const XTERM_MARKER: &str = "xterm-helper-textarea";

    #[test]
    fn xterm_ships_as_its_own_module_under_the_console_cap() {
        assert!(
            XTERM_JS.contains(XTERM_MARKER),
            "built xterm.js looks wrong"
        );
        assert!(
            XTERM_JS.len() < 8 * 1024 * 1024,
            "{XTERM_PATH} is {} bytes — past the console's 8 MiB asset cap",
            XTERM_JS.len()
        );
    }

    /// xterm is loaded on demand; a value import of `@xterm/*` anywhere in
    /// page.tsx's graph bundles it back into every tab's first load.
    #[test]
    fn page_does_not_bundle_xterm() {
        assert!(
            !PAGE_JS.contains(XTERM_MARKER),
            "page.js bundles xterm — import it only through host.importModule"
        );
        assert!(
            PAGE_JS.contains(XTERM_PATH),
            "page.js must import xterm from {XTERM_PATH}"
        );
    }
}
