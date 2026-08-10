fn main() {
    // Forwards the build-time target triple as `env!("TARGET")` — used by
    // `manifest.rs` for the registry `supported_targets` field. Same
    // convention every other worker's build.rs uses.
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
