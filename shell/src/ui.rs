//! Injectable console UI for the shell worker
//! (iii/tech-specs/2026-07-17-injectable-ui; authoring SOP:
//! workers/docs/sops/injectable-console-ui.md).
//!
//! Ships two assets into any running console:
//!
//! - `shell/page.js` (`console:script`) — the shell explorer page
//!   (#/ext/shell: file tree / git / search sidebar beside the shared
//!   Monaco editor and FileDiff pane), plus the `shell::*`
//!   function-trigger renderer its `setup(host)` registers (moved out of
//!   the console SPA, the iii-directory precedent).
//! - `shell/styles.css` (`console:style`) — the stylesheet, every rule
//!   scoped under `[data-iii-ui="shell"]`.
//!
//! The registration machinery (content function `shell::ui-content`, one
//! Message-path trigger per asset, `III_SHELL_UI_WATCH` hot-reload
//! watcher) lives in the shared `iii-console-ui` crate (path-linked from
//! `workers/crates/console-ui`); this module only names the assets and
//! embeds their bytes.
//!
//! The assets are compiled from `ui/` by esbuild (react +
//! @iii-dev/console-ui external — they resolve through the console's
//! import map at runtime) and embedded at compile time so the worker
//! stays one self-contained binary. For the dev loop, set
//! `III_SHELL_UI_WATCH` to the build output directory (or `1` for
//! `ui/dist`): the worker polls both files and re-registers a changed
//! asset's trigger — every open console tab hot-swaps it.

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

pub const PAGE_PATH: &str = "shell/page.js";
pub const STYLES_PATH: &str = "shell/styles.css";
/// Mirrored by `ui/src/page/persist.ts` (`UI_STATE_CONFIG_ID`).
const UI_STATE_CONFIG_ID: &str = "shell-ui";

/// Built by `build.rs` (esbuild over `ui/`).
const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

fn console_ui() -> ConsoleUi {
    ConsoleUi::new("shell")
        .script(PAGE_PATH, PAGE_JS)
        .style(STYLES_PATH, STYLES_CSS)
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
    register_ui_state_entry(iii.clone());
}

/// The `shell-ui` configuration entry backs the explorer page's per-pane
/// UI state (browsed folder, open editor tabs, expanded folders):
/// `{ tabs: { [paneId]: {...} } }`, read-modify-written by the page over
/// `configuration::get`/`set`. Registered so the entry exists before the
/// first `set`.
///
/// The `{}` seed is installed ONLY when nothing is stored yet (entry
/// missing or value null): `configuration::register` replaces the stored
/// value whenever `initial_value` is present, not only on first
/// registration, so an unconditional seed erased every pane's folder and
/// tabs on each worker restart. If the probe itself fails the entry is
/// registered without a seed — never clobber what may be there.
/// Fire-and-forget: a missing configuration worker degrades the page to
/// non-persistent, never blocks the worker.
fn register_ui_state_entry(iii: IIIClient) {
    tokio::spawn(async move {
        let mut payload = serde_json::json!({
            "id": UI_STATE_CONFIG_ID,
            "name": "Shell UI",
            "description": "Per-pane state for the shell explorer page \
                            (browsed folder, open files, expanded folders). \
                            Managed by the page; not intended for hand-editing.",
            "schema": { "type": "object", "additionalProperties": true },
        });
        match crate::configuration::stored_value_absent_for(&iii, UI_STATE_CONFIG_ID).await {
            Ok(true) => payload["initial_value"] = serde_json::json!({}),
            Ok(false) => {}
            Err(e) => tracing::warn!(
                error = %e,
                "could not probe the shell-ui configuration entry; registering it without a seed"
            ),
        }
        if let Err(e) = crate::configuration::trigger_configuration_with_retry(
            &iii,
            "configuration::register",
            payload,
        )
        .await
        {
            tracing::warn!(
                error = %e,
                "shell-ui configuration entry not registered (explorer UI state won't persist)"
            );
        }
    });
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
        // ([data-iii-ui=shell]).
        assert!(
            STYLES_CSS.contains(r#"[data-iii-ui="shell"]"#)
                || STYLES_CSS.contains("[data-iii-ui=shell]"),
            "built styles.css must be scoped under the worker's data-iii-ui attribute"
        );
    }
}
