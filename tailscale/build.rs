#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET is set by Cargo")
    );

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
