//! Build script for the `compose-ui` worker.
//!
//! 1. Forwards the build-time target triple to the binary.
//! 2. Ensures the injected Console UI assets exist. `src/ui.rs` embeds
//!    `ui/dist/page.js` and `ui/dist/styles.css`, so missing or stale assets
//!    are rebuilt with pnpm before Rust compilation. Set `SKIP_UI_BUILD=1`
//!    only when both generated assets already exist.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").expect("Cargo sets TARGET for build scripts")
    );

    // Never watch dist itself: include_str! reads it directly, and watching
    // our own output would create a rebuild loop.
    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let assets = [
        ui_dir.join("dist").join("page.js"),
        ui_dir.join("dist").join("styles.css"),
    ];

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
                "SKIP_UI_BUILD set but {} is missing — run `pnpm --dir ui build` or unset SKIP_UI_BUILD",
                asset.display()
            );
        }
        return;
    }

    let pnpm = locate_pnpm();
    for command in ["install", "build"] {
        let status = Command::new(&pnpm)
            .arg(command)
            .current_dir(&ui_dir)
            .status()
            .unwrap_or_else(|error| {
                panic!(
                    "failed to spawn `pnpm {}` in {}: {error}",
                    command,
                    ui_dir.display()
                )
            });
        assert!(
            status.success(),
            "`pnpm {}` exited with {status} — see logs above",
            command
        );
    }

    for asset in &assets {
        assert!(
            asset.exists(),
            "`pnpm build` finished but {} is missing",
            asset.display()
        );
    }
}

fn dist_is_fresh(asset: &Path, ui_dir: &Path) -> bool {
    let Ok(asset_mtime) = asset.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };

    for source in [
        ui_dir.join("page.tsx"),
        ui_dir.join("styles.css"),
        ui_dir.join("build.mjs"),
        ui_dir.join("package.json"),
        ui_dir.join("tsconfig.json"),
        ui_dir.join("../../pnpm-lock.yaml"),
    ] {
        if !source.exists() {
            continue;
        }
        let Ok(source_mtime) = source.metadata().and_then(|metadata| metadata.modified()) else {
            return false;
        };
        if source_mtime > asset_mtime {
            return false;
        }
    }

    subtree_older_than(&ui_dir.join("src"), asset_mtime)
}

fn subtree_older_than(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        if metadata.is_dir() {
            if !subtree_older_than(&entry.path(), ceiling) {
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
    let candidates: &[&str] = if cfg!(windows) {
        &["pnpm.cmd", "pnpm.exe", "pnpm"]
    } else {
        &["pnpm"]
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
        "pnpm not found on PATH — install pnpm, set PNPM, or set SKIP_UI_BUILD=1 after building ui/"
    );
}
