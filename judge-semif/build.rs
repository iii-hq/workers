//! Build script for the judge-semif worker: llama.cpp's runtime backends and
//! the injectable console UI.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    llama_runtime();
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CONSOLE_UI");
    if std::env::var_os("CARGO_FEATURE_CONSOLE_UI").is_none() {
        return;
    }

    println!("cargo:rerun-if-env-changed=SKIP_UI_BUILD");
    println!("cargo:rerun-if-env-changed=PNPM");
    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ui_dir = manifest_dir.join("ui");
    let dist_assets = [
        ui_dir.join("dist").join("page.js"),
        ui_dir.join("dist").join("styles.css"),
    ];

    if dist_assets
        .iter()
        .all(|asset| asset.exists() && dist_is_fresh(asset, &ui_dir))
    {
        return;
    }

    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        for asset in &dist_assets {
            if !asset.exists() {
                panic!(
                    "SKIP_UI_BUILD set but {} is missing; build the UI first",
                    asset.display()
                );
            }
        }
        return;
    }

    let pnpm = locate_pnpm();
    run(&pnpm, &["install"], &ui_dir);
    run(&pnpm, &["build"], &ui_dir);

    for asset in &dist_assets {
        if !asset.exists() {
            panic!("UI build completed but {} is missing", asset.display());
        }
    }
}

fn run(command: &Path, args: &[&str], cwd: &Path) {
    let status = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", command.display()));
    if !status.success() {
        panic!("{} exited with {status}", command.display());
    }
}

fn dist_is_fresh(dist_asset: &Path, ui_dir: &Path) -> bool {
    let Ok(dist_mtime) = dist_asset.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    let watched_files = [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
        ui_dir.join("tsconfig.json"),
    ];
    for file in watched_files {
        if !file.exists() {
            continue;
        }
        let Ok(modified) = file.metadata().and_then(|meta| meta.modified()) else {
            return false;
        };
        if modified > dist_mtime {
            return false;
        }
    }
    subtree_older_than(&ui_dir.join("src"), dist_mtime)
}

fn subtree_older_than(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            return false;
        };
        if meta.is_dir() {
            if !subtree_older_than(&path, ceiling) {
                return false;
            }
        } else {
            let Ok(modified) = meta.modified() else {
                return false;
            };
            if modified > ceiling {
                return false;
            }
        }
    }
    true
}

fn locate_pnpm() -> PathBuf {
    if let Ok(explicit) = std::env::var("PNPM") {
        return PathBuf::from(explicit);
    }
    let names = if cfg!(windows) {
        ["pnpm.cmd", "pnpm.exe", "pnpm"].as_slice()
    } else {
        ["pnpm"].as_slice()
    };
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("pnpm not found on PATH; set PNPM or build judge-semif/ui manually");
}

/// Linux x86_64 and Windows builds link llama.cpp as shared libraries and load
/// its backends at runtime (Cargo.toml). Give the build directory the published
/// layout: the backend modules and the shared libraries the binary needs, next
/// to it. llama-cpp-sys-2 only links the unversioned `*.so` symlinks there,
/// whose `*.so.0` targets the Linux binary actually loads, through `$ORIGIN`.
fn llama_runtime() {
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
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        let libraries = backends
            .parent()
            .expect("backends has a parent")
            .join("lib");
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
    // Drop the symbols the loader does not need.
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
