//! Build script for the `image-resize` worker: builds the injected console UI
//! that `src/ui.rs` embeds via `include_str!` when it is missing or stale. Set
//! `SKIP_UI_BUILD=1` to use the existing `ui/dist/` outputs as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
