use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_init_binary)");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("iii-init");

    if !cfg!(feature = "embed-init") {
        // Feature disabled: write a single zero byte as placeholder.
        std::fs::write(&dest, [0u8]).expect("failed to write iii-init placeholder");
        return;
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // VMs always run Linux guests, so init is always a Linux musl binary
    // regardless of the host OS (macOS uses the same Linux guest arch).
    let triple = match arch.as_str() {
        "x86_64" => "x86_64-unknown-linux-musl",
        "aarch64" => "aarch64-unknown-linux-musl",
        other => panic!("embed-init: no guest init for target arch {other:?}"),
    };

    // Where scripts/build-init.sh left it: that script runs cargo from the
    // workspace root, so a relative CARGO_TARGET_DIR is relative to it too.
    println!("cargo:rerun-if-env-changed=CARGO_TARGET_DIR");
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || workspace_root.join("target"),
        |dir| workspace_root.join(dir),
    );
    let binary_path = target_dir.join(triple).join("release").join("iii-init");

    // Track the cross-compiled binary and every source it was built from,
    // not a source-tree allowlist: iii-init links iii-supervisor and
    // iii-shell-proto, and an allowlist that missed one of them once let a
    // stale init ship. `rerun-if-changed` on a missing path reruns when it
    // appears.
    println!("cargo:rerun-if-changed={}", binary_path.display());
    let built = modified(&binary_path).unwrap_or_else(|| {
        panic!(
            "embed-init: no guest init at {}; run scripts/build-init.sh first",
            binary_path.display()
        )
    });
    // cargo's dep-info next to the binary lists the sources it compiled.
    let dep_info_path = binary_path.with_extension("d");
    let dep_info = std::fs::read_to_string(&dep_info_path).unwrap_or_else(|e| {
        panic!(
            "embed-init: {}: {e}; run scripts/build-init.sh again",
            dep_info_path.display()
        )
    });
    for source in dep_info_sources(&dep_info) {
        println!("cargo:rerun-if-changed={source}");
        // A source that is gone was dropped by an edit to one that is not.
        if modified(Path::new(&source)).is_some_and(|edited| edited > built) {
            panic!(
                "embed-init: {} is older than {source}; run scripts/build-init.sh again",
                binary_path.display()
            );
        }
    }

    std::fs::copy(&binary_path, &dest).expect("failed to copy iii-init to OUT_DIR");
    println!("cargo:rustc-cfg=has_init_binary");
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The prerequisites of a Makefile-style dep-info file (`target: a b\ c`).
fn dep_info_sources(dep_info: &str) -> Vec<String> {
    let mut sources = Vec::new();
    for line in dep_info.lines() {
        let Some((_, deps)) = line.split_once(": ") else {
            continue;
        };
        let mut source = String::new();
        let mut chars = deps.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => source.extend(chars.next()),
                ' ' if !source.is_empty() => sources.push(std::mem::take(&mut source)),
                ' ' => {}
                c => source.push(c),
            }
        }
        if !source.is_empty() {
            sources.push(source);
        }
    }
    sources
}
