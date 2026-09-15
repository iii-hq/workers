fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("TARGET must be set by Cargo build scripts")
    );
}
