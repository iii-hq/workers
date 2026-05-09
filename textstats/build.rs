// Build script. Exposes the build target triple as an env var for the worker
// binary. Not consumed by iii-skill-check.
fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
