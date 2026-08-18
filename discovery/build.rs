//! Build the discovery worker's injectable console assets and forward TARGET.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    println!("cargo:rerun-if-env-changed=SKIP_UI_BUILD");
    println!("cargo:rerun-if-env-changed=PNPM");
    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let assets = [ui_dir.join("dist/page.js"), ui_dir.join("dist/styles.css")];
    if assets
        .iter()
        .all(|asset| asset.exists() && dist_is_fresh(asset, &ui_dir))
    {
        return;
    }

    if std::env::var_os("SKIP_UI_BUILD").is_some() {
        for asset in &assets {
            assert!(
                asset.exists(),
                "SKIP_UI_BUILD set but {} is missing; build discovery/ui first",
                asset.display()
            );
        }
        return;
    }

    let pnpm = locate_pnpm();
    let install = Command::new(&pnpm)
        .arg("install")
        .current_dir(&ui_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to run pnpm install in {}: {e}", ui_dir.display()));
    assert!(install.success(), "pnpm install failed with {install}");

    let build = Command::new(&pnpm)
        .arg("build")
        .current_dir(&ui_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to build discovery UI in {}: {e}", ui_dir.display()));
    assert!(build.success(), "discovery UI build failed with {build}");
    for asset in &assets {
        assert!(
            asset.exists(),
            "UI build did not create {}",
            asset.display()
        );
    }
}

fn dist_is_fresh(asset: &Path, ui_dir: &Path) -> bool {
    let Ok(built_at) = asset.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    for source in [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
        ui_dir.join("tsconfig.json"),
    ] {
        let Ok(modified) = source.metadata().and_then(|meta| meta.modified()) else {
            return false;
        };
        if modified > built_at {
            return false;
        }
    }
    subtree_older_than(&ui_dir.join("src"), built_at)
}

fn subtree_older_than(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            return false;
        };
        if meta.is_dir() {
            if !subtree_older_than(&entry.path(), ceiling) {
                return false;
            }
        } else if meta.modified().map_or(true, |modified| modified > ceiling) {
            return false;
        }
    }
    true
}

fn locate_pnpm() -> PathBuf {
    if let Ok(path) = std::env::var("PNPM") {
        return PathBuf::from(path);
    }
    let names = if cfg!(windows) {
        ["pnpm.cmd", "pnpm.exe", "pnpm"].as_slice()
    } else {
        ["pnpm"].as_slice()
    };
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("pnpm not found on PATH; set PNPM or build discovery/ui manually");
}
