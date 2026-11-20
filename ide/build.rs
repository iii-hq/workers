//! Build script for the `shell` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`.
//! 2. Builds the injected console UI that `src/ui.rs` embeds via
//!    `include_str!` (page, styles and the xterm module) when it is missing
//!    or stale. Set `SKIP_UI_BUILD=1` to use the existing `ui/dist/` outputs
//!    as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );

    ui_build::ensure_built(&ui_build::Frontend {
        dir: "ui",
        assets: &["dist/page.js", "dist/styles.css", "dist/xterm.js"],
        // The terminal and editor bundle both prefs packages.
        packages: &[
            "packages/console-ui",
            "packages/terminal-font",
            "packages/editor-prefs",
        ],
        extra: &[],
        skip_env: "SKIP_UI_BUILD",
    });
}
