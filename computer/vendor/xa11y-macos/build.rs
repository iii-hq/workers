fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=src/exception_safe.m");

        cc::Build::new()
            .file("src/exception_safe.m")
            .flag("-fobjc-exceptions")
            .flag("-fobjc-arc")
            .flag("-fmodules")
            .compile("exception_safe");

        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
    }
}
