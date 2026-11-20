//! Build script for the `iii-directory` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Builds the injected console UI that `src/ui.rs` embeds (`ui_build`).

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

/// Targets with a pinned static ONNX Runtime in ort-sys' dist.tsv. Mirrors
/// the `[target.'cfg(...)'.dependencies]` table in Cargo.toml; the code gates
/// MiniLM on `cfg(minilm)` so the two never drift apart in 18 places.
const MINILM_TARGETS: [&str; 5] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

fn main() {
    let target = std::env::var("TARGET").unwrap();
    println!("cargo:rustc-env=TARGET={target}");
    println!("cargo:rustc-check-cfg=cfg(minilm)");
    if MINILM_TARGETS.contains(&target.as_str()) {
        println!("cargo:rustc-cfg=minilm");
    }

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
