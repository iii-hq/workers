//! Build script for the `ade` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Builds the embedded SPA bundle when missing or stale (`ui_build`):
//!    `rust-embed` reads `web/dist/` at compile time. Set `SKIP_WEB_BUILD=1`
//!    to use the existing `web/dist/` as-is.
//! 3. Builds the ADE worker's *own* injected UI assets the same way:
//!    `src/ui.rs` embeds `ui/dist/config-form.js`, `ui/dist/catalog-page.js`,
//!    `ui/dist/workspace-proposal.js`, and `ui/dist/styles.css` via
//!    `include_str!`. Set `SKIP_UI_BUILD=1` to use the existing `ui/dist/`.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    ui_build::ensure_built(&ui_build::Frontend {
        dir: "web",
        assets: &["dist/index.html"],
        packages: &["packages/traces-ui", "packages/console-ui"],
        // ProviderMark imports the cursor worker's and the providers' icons.
        extra: &[
            "web/public",
            "web/.env.production",
            "../cursor/src/provider-icon.ts",
            "../provider-anthropic/assets/icon.svg",
            "../provider-claude-code/assets/icon.svg",
            "../provider-command-code/assets/icon.svg",
            "../provider-deepseek/assets/icon.svg",
            "../provider-github-copilot/assets/icon.svg",
            "../provider-kimi/assets/icon.svg",
            "../provider-llamacpp/assets/icon.svg",
            "../provider-openai/assets/icon.svg",
            "../provider-opencode-go/assets/icon.svg",
            "../provider-openrouter/assets/icon.svg",
            "../provider-xai/assets/icon.svg",
            "../provider-zai/assets/icon.svg",
        ],
        skip_env: "SKIP_WEB_BUILD",
    });
    ui_build::ensure_built(&ui_build::Frontend {
        dir: "ui",
        assets: &[
            "dist/config-form.js",
            "dist/catalog-page.js",
            "dist/workspace-proposal.js",
            "dist/styles.css",
        ],
        // The IDE editor section bundles both prefs packages.
        packages: &[
            "packages/console-ui",
            "packages/editor-prefs",
            "packages/terminal-font",
        ],
        extra: &[],
        skip_env: "SKIP_UI_BUILD",
    });
}
