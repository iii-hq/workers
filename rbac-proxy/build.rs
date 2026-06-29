//! Build script for the `rbac-proxy` worker.
//!
//! Forwards the build-time target triple to the binary as `env!("TARGET")`,
//! consumed by `manifest.rs` for the registry `supported_targets` field.
//! Unlike `console`, this worker embeds no web assets, so there is nothing
//! else to do here.

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
