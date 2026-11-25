//! Build script for the `artifact` worker: forwards the build-time target
//! triple to the binary as `env!("TARGET")`, used by `manifest.rs` for the
//! registry `supported_targets` field.

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
