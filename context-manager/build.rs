//! Build script for the `context-manager` worker.
//!
//! Forwards the target triple to the manifest builder and ensures the
//! injectable console UI assets exist before `src/ui.rs` embeds them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");
    println!("cargo:rerun-if-env-changed=SKIP_UI_BUILD");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
                    "SKIP_UI_BUILD set but {} is missing — build the UI manually \
                     (cd ui && pnpm install && pnpm build) or unset the env var",
                    asset.display()
                );
            }
        }
        return;
    }

    let pnpm = locate_pnpm();
    for args in [["install"].as_slice(), ["build"].as_slice()] {
        let status = Command::new(&pnpm)
            .args(args)
            .current_dir(&ui_dir)
            .status()
            .unwrap_or_else(|error| {
                panic!(
                    "failed to spawn `pnpm {}` in {}: {error}",
                    args.join(" "),
                    ui_dir.display()
                )
            });
        if !status.success() {
            panic!(
                "`pnpm {}` exited with {status} — see logs above",
                args.join(" ")
            );
        }
    }

    for asset in &dist_assets {
        if !asset.exists() {
            panic!(
                "`pnpm build` finished but {} is still missing — check the esbuild output above",
                asset.display()
            );
        }
    }
}

fn dist_is_fresh(dist_asset: &Path, ui_dir: &Path) -> bool {
    let Ok(dist_mtime) = dist_asset.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };

    for file in [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
        ui_dir.join("tsconfig.json"),
    ] {
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
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            if !subtree_older_than(&path, ceiling) {
                return false;
            }
        } else {
            let Ok(modified) = metadata.modified() else {
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
    let candidates = if cfg!(windows) {
        ["pnpm.cmd", "pnpm.exe", "pnpm"].as_slice()
    } else {
        ["pnpm"].as_slice()
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        for name in candidates {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!(
        "pnpm not found on PATH — install Node + pnpm, or set SKIP_UI_BUILD=1 \
         after building the UI manually with `cd ui && pnpm install && pnpm build`"
    );
}
