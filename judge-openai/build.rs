//! Build script for the OpenAI judge worker's injectable console UI.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CONSOLE_UI");
    if std::env::var_os("CARGO_FEATURE_CONSOLE_UI").is_none() {
        return;
    }

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
