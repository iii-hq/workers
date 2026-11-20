//! Build script for the judge-clef worker: the `$ORIGIN` runpath, then the
//! injectable console UI.

#[path = "../crates/console-ui/src/ui_build.rs"]
mod ui_build;

fn main() {
    // Linux finds llama.cpp's shared libraries (laid beside the binary by
    // crates/llama-native) through the binary's own runpath.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CONSOLE_UI");
    if std::env::var_os("CARGO_FEATURE_CONSOLE_UI").is_none() {
        return;
    }
    ui_build::ensure_console_ui(&["dist/page.js", "dist/styles.css"]);
}
