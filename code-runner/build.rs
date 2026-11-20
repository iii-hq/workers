//! Build script for the `code-runner` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Builds the injected console UI that `src/ui.rs` embeds via
//!    `include_str!` when it is missing or stale. Set `SKIP_UI_BUILD=1` to
//!    use the existing `ui/dist/` outputs as-is.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    // Only the two console assets: unlike sandbox-code-runner, this worker
    // plants no `iii-sdk` bundle into a guest — its Node guest gets a
    // host-implemented op surface and its Python guest a framed stdio bridge,
    // so there is nothing to bundle for them.
    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
