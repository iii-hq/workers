//! Lay llama.cpp's runtime pieces beside the binaries that link this crate.
//!
//! With `dynamic-backends` (Linux x86_64, Windows) llama-cpp-sys-2 builds the
//! backends as modules and the core as shared libraries; this script copies
//! them into the profile directory (the published layout). Linker arguments do
//! not reach a dependent's binary from here: each worker's build.rs adds the
//! `$ORIGIN` runpath itself.
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DEP_LLAMA_BACKENDS_DIR");
    let Some(backends) = std::env::var_os("DEP_LLAMA_BACKENDS_DIR").map(PathBuf::from) else {
        return;
    };
    // OUT_DIR is <profile>/build/<crate>-<hash>/out.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR lies under the profile directory")
        .to_path_buf();
    // Tests run from deps/ and examples from examples/: they need the libraries,
    // and find the modules through llama-cpp-2's build-time fallback.
    let library_dirs = [
        profile_dir.clone(),
        profile_dir.join("deps"),
        profile_dir.join("examples"),
    ];
    let linux = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    // Release copies are what ships: strip them like the binary (`strip = true`).
    let strip = linux && std::env::var("PROFILE").as_deref() == Ok("release");
    for module in files(&backends) {
        copy_into(&module, &profile_dir, strip);
    }
    if linux {
        // CMake's GNUInstallDirs picks lib64 on some distributions (Fedora).
        let root = backends.parent().expect("backends has a parent");
        let libraries = ["lib", "lib64"]
            .map(|dir| root.join(dir))
            .into_iter()
            .find(|dir| dir.is_dir())
            .expect("llama.cpp installs its libraries under lib or lib64");
        for library in files(&libraries) {
            let name = library
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            // The SONAME files (`libllama.so.0`), all of them: llama-cpp-sys-2
            // re-links its unversioned symlinks here on every run and fails
            // on one left dangling.
            let soname = name.split_once(".so.").is_some_and(|(_, version)| {
                !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
            });
            if soname {
                for dir in library_dirs.iter().filter(|dir| dir.is_dir()) {
                    copy_into(&library, dir, strip);
                }
            }
        }
    }
}

fn files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.expect("directory entry is readable").path())
        .collect()
}

/// Copy the file's contents (following symlinks) under its own name; `strip`
/// removes the symbols the loader does not need (release builds).
fn copy_into(path: &Path, dir: &Path, strip: bool) {
    let destination = dir.join(path.file_name().expect("file has a name"));
    let _ = std::fs::remove_file(&destination);
    std::fs::copy(path, &destination).unwrap_or_else(|error| {
        panic!(
            "copy {} to {}: {error}",
            path.display(),
            destination.display()
        )
    });
    if strip
        && !Command::new("strip")
            .arg("--strip-unneeded")
            .arg(&destination)
            .status()
            .is_ok_and(|status| status.success())
    {
        println!("cargo:warning=could not strip {}", destination.display());
    }
}
