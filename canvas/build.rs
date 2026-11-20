//! Build script for the `canvas` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Builds the injected console UI that `src/ui.rs` embeds via
//!    `include_str!` (page, styles, mermaid and freeform bundles) when it is
//!    missing or stale. Set `SKIP_UI_BUILD=1` to use the existing `ui/dist/`
//!    outputs as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    ui_build::ensure_console_ui(&[
        "dist/page.js",
        "dist/styles.css",
        "dist/mermaid.js",
        "dist/freeform.js",
    ]);
}
