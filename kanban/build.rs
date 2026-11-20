//! Build script for the `kanban` worker.
//!
//! Builds the embedded frontends when missing or stale (`ui_build`):
//! - `src/ui.rs` embeds the injected console UI, `ui/dist/page.js` and
//!   `ui/dist/styles.css`;
//! - `src/native_ui.rs` embeds the native app for the iii app,
//!   `native-ui/dist/native.js` (which bundles the `@iii-dev/native-ui` SDK).
//!
//! Set `SKIP_UI_BUILD=1` to use the existing `dist/` outputs of both as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
    ui_build::ensure_built(&ui_build::Frontend {
        dir: "native-ui",
        assets: &["dist/native.js"],
        packages: &["packages/native-ui"],
        extra: &[],
        skip_env: "SKIP_UI_BUILD",
    });
}
