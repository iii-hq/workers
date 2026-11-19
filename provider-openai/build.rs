#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET must be set by Cargo build scripts")
    );

    if std::env::var_os("CARGO_FEATURE_CONSOLE_UI").is_none() {
        return;
    }

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
