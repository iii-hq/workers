//! Build script for the `llm-router` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Ensures the injected console UI assets exist: `src/ui.rs` embeds
//!    `ui/dist/page.js` and `ui/dist/styles.css` via `include_str!`, so if
//!    either is missing or stale we run `pnpm install && pnpm build` inside
//!    `ui/` first. Set `SKIP_UI_BUILD=1` to use the existing `ui/dist/`
//!    outputs as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET must be set by Cargo build scripts")
    );

    // Everything below builds/validates the injectable console UI bundle.
    // Skip it when the `console-ui` feature is off (lib-only path dependents)
    // so their builds never need a Node toolchain.
    if std::env::var_os("CARGO_FEATURE_CONSOLE_UI").is_none() {
        return;
    }

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
