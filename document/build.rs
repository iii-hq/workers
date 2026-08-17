//! Build script for the `document` worker.
//!
//! One job: forward the build-time target triple to the binary as
//! `env!("TARGET")`, which `manifest.rs` reports as the registry's
//! `supported_targets` field.

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
