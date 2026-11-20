//! Build script for the `database` worker.
//!
//! `src/ui.rs` embeds `ui/dist/page.js` and `ui/dist/styles.css` via
//! `include_str!`; the shared helper rebuilds them when missing or stale.
//! Set `SKIP_UI_BUILD=1` to use the existing `ui/dist/` outputs as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
