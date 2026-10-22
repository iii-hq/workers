use std::path::PathBuf;

include!("build/firmware_check.rs");

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_libkrunfw)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/firmware_check.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // --- libkrunfw embedding ---
    // The VMM (msb_krun) is compiled as a Rust dependency -- no external libkrun needed.
    // Only libkrunfw (guest kernel firmware) requires embedding.
    let libkrunfw_dest = out_dir.join("libkrunfw");
    if cfg!(feature = "embed-libkrunfw") {
        let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let (target_os, ext) = if os == "macos" {
            ("darwin", "dylib")
        } else {
            ("linux", "so")
        };

        let file_name = format!("libkrunfw-{target_os}-{arch}.{ext}");
        let fw_path = crate_dir.join("firmware").join(&file_name);
        let sums_path = crate_dir.join("firmware").join("SHA256SUMS");
        println!("cargo:rerun-if-changed={}", fw_path.display());
        println!("cargo:rerun-if-changed={}", sums_path.display());

        // Fail closed: a missing, LFS-pointer or tampered firmware must never ship as a 1-byte stub.
        let bytes = std::fs::read(&fw_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", fw_path.display()));
        let sums = std::fs::read_to_string(&sums_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", sums_path.display()));
        check_firmware(&file_name, &bytes, &sums).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&libkrunfw_dest, &bytes).expect("failed to write libkrunfw to OUT_DIR");
    } else {
        std::fs::write(&libkrunfw_dest, [0u8]).expect("failed to write libkrunfw placeholder");
    }
    if std::fs::metadata(&libkrunfw_dest).is_ok_and(|m| m.len() > 1) {
        println!("cargo:rustc-cfg=has_libkrunfw");
    }
}
