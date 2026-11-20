//! Build script for the `sandbox-code-runner` worker.
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

    ui_build::ensure_console_ui(&[
        "dist/page.js",
        "dist/styles.css",
        // The guest SDK bundle `src/runner.rs` embeds and plants into every
        // Node runtime (+ its generated package manifest) — built by the
        // same `pnpm build`, from the pinned `iii-sdk` dependency.
        "dist/iii-sdk-guest.mjs",
        "dist/iii-sdk-guest-package.json",
    ]);
}
