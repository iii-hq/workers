#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=TARGET={target}");
    }

    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
