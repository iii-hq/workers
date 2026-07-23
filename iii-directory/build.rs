//! Build script for the `iii-directory` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Ensures the injected console UI assets exist: `src/ui.rs` embeds
//!    `ui/dist/page.js` and `ui/dist/styles.css` via `include_str!`, so if
//!    either is missing or stale we run `pnpm install && pnpm build` inside
//!    `ui/` first (the state worker's precedent). Set `SKIP_UI_BUILD=1` to
//!    use the existing `ui/dist/` outputs as-is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    // `dist/` itself is not listed: include_str! reads it directly, and
    // listing it would rebuild-loop on our own output.
    println!("cargo:rerun-if-changed=ui/page.tsx");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/build.mjs");
    println!("cargo:rerun-if-changed=ui/package.json");
    // The lockfile lives at the workers-repo root (pnpm workspace: the ui
    // project links @iii-dev/console-ui from packages/console-ui).
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");
    let dist_assets = [
        ui_dir.join("dist").join("page.js"),
        ui_dir.join("dist").join("styles.css"),
    ];

    if dist_assets
        .iter()
        .all(|a| a.exists() && dist_is_fresh(a, &ui_dir))
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

    let status = Command::new(&pnpm)
        .args(["install"])
        .current_dir(&ui_dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "failed to spawn `pnpm install` in {}: {e}",
                ui_dir.display()
            )
        });
    if !status.success() {
        panic!("`pnpm install` exited with {status} — see logs above");
    }

    let status = Command::new(&pnpm)
        .args(["build"])
        .current_dir(&ui_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `pnpm build` in {}: {e}", ui_dir.display()));
    if !status.success() {
        panic!("`pnpm build` exited with {status} — see logs above");
    }

    for asset in &dist_assets {
        if !asset.exists() {
            panic!(
                "`pnpm build` finished but {} is still missing — check the esbuild \
                 output above",
                asset.display()
            );
        }
    }
}

/// `true` when the built asset is at least as new as every source that
/// contributes to it. Conservative: any I/O failure forces a rebuild.
fn dist_is_fresh(dist_asset: &Path, ui_dir: &Path) -> bool {
    let Ok(dist_mtime) = dist_asset.metadata().and_then(|m| m.modified()) else {
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
    for f in watched_files.iter() {
        if !f.exists() {
            continue;
        }
        let Ok(m) = f.metadata().and_then(|m| m.modified()) else {
            return false;
        };
        if m > dist_mtime {
            return false;
        }
    }

    for dir in [ui_dir.join("src")] {
        if dir.exists() && !subtree_older_than(&dir, dist_mtime) {
            return false;
        }
    }

    true
}

fn subtree_older_than(root: &Path, ceiling: SystemTime) -> bool {
    let Ok(read) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            return false;
        };
        if meta.is_dir() {
            if !subtree_older_than(&path, ceiling) {
                return false;
            }
        } else {
            let Ok(m) = meta.modified() else {
                return false;
            };
            if m > ceiling {
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
    for dir in std::env::split_paths(&path) {
        for name in candidates {
            let candidate = dir.join(name);
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
