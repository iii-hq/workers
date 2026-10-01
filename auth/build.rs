fn main() {
    let target = std::env::var("TARGET").expect("TARGET must be set by Cargo for auth/build.rs");
    println!("cargo:rustc-env=TARGET_TRIPLE={}", target);
}
