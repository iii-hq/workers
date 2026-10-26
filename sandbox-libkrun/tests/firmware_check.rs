include!("../build/firmware_check.rs");

#[test]
fn firmware_check_rejects_an_lfs_pointer_and_a_wrong_sum() {
    let sums = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/firmware/SHA256SUMS"))
        .unwrap();
    let real = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/firmware/libkrunfw-linux-x86_64.so"
    ))
    .unwrap();
    assert!(check_firmware("libkrunfw-linux-x86_64.so", &real, &sums).is_ok());
    let lfs = b"version https://git-lfs.github.com/spec/v1\n";
    assert!(
        check_firmware("libkrunfw-linux-x86_64.so", lfs, &sums)
            .unwrap_err()
            .contains("not an ELF")
    );
    let mut tampered = real.clone();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(
        check_firmware("libkrunfw-linux-x86_64.so", &tampered, &sums)
            .unwrap_err()
            .contains("SHA256SUMS")
    );
}

#[test]
fn firmware_check_handles_dylib_and_unlisted_files() {
    let sums = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/firmware/SHA256SUMS"))
        .unwrap();
    let dylib = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/firmware/libkrunfw-darwin-aarch64.dylib"
    ))
    .unwrap();
    assert!(check_firmware("libkrunfw-darwin-aarch64.dylib", &dylib, &sums).is_ok());
    // An ELF is not a Mach-O, and a file without a SHA256SUMS line is refused.
    assert!(
        check_firmware("libkrunfw-darwin-aarch64.dylib", b"\x7fELF", &sums)
            .unwrap_err()
            .contains("Mach-O")
    );
    assert!(
        check_firmware("libkrunfw-linux-riscv64.so", b"\x7fELF", &sums)
            .unwrap_err()
            .contains("no entry")
    );
}
