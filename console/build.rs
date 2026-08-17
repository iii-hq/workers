//! Build script for the `console` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Ensures the embedded SPA bundle exists. `rust-embed` reads
//!    `web/dist/` at compile time; if it's missing or stale we run
//!    `pnpm install --frozen-lockfile && pnpm build` inside `web/`
//!    before the rest of the crate compiles.
//! 3. Ensures the console's *own* injected UI assets exist: `src/ui.rs`
//!    embeds `ui/dist/config-form.js`, `ui/dist/catalog-page.js`, and
//!    `ui/dist/styles.css` via `include_str!` (the state worker precedent).
//!    Set `SKIP_UI_BUILD=1` to use the existing `ui/dist/` outputs as-is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    ensure_web_bundle();
    ensure_ui_assets();
}

fn ensure_web_bundle() {
    // Trigger a re-run when the JS source or its toolchain inputs change.
    // `dist/` itself is *not* listed: rust-embed reads it directly, and
    // listing it would create a dependency cycle (touching dist forces a
    // rebuild that touches dist again).
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-changed=web/tsconfig.app.json");
    println!("cargo:rerun-if-changed=web/tsconfig.json");
    println!("cargo:rerun-if-changed=web/tsconfig.node.json");
    println!("cargo:rerun-if-changed=web/.env.production");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let web_dir = manifest_dir.join("web");
    let dist_dir = web_dir.join("dist");
    let dist_index = dist_dir.join("index.html");

    if dist_index.exists() && dist_is_fresh(&dist_index, &web_dir) {
        return;
    }

    if std::env::var_os("SKIP_WEB_BUILD").is_some() {
        if !dist_index.exists() {
            panic!(
                "SKIP_WEB_BUILD set but {} is missing — build the \
                 SPA manually (cd web && pnpm install && pnpm build) or \
                 unset the env var",
                dist_index.display()
            );
        }
        return;
    }

    let pnpm = locate_pnpm();

    let status = Command::new(&pnpm)
        .args(["install", "--frozen-lockfile"])
        .current_dir(&web_dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "failed to spawn `pnpm install` in {}: {e}",
                web_dir.display()
            )
        });
    if !status.success() {
        panic!("`pnpm install` exited with {status} — see logs above");
    }

    let status = Command::new(&pnpm)
        .args(["build"])
        .current_dir(&web_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `pnpm build` in {}: {e}", web_dir.display()));
    if !status.success() {
        panic!("`pnpm build` exited with {status} — see logs above");
    }

    if !dist_index.exists() {
        panic!(
            "`pnpm build` finished but {} is still missing — check the \
             vite output above",
            dist_index.display()
        );
    }
}

/// The injected-UI assets (`ui/dist/*`, embedded by `src/ui.rs`): same
/// contract as the state worker's `build.rs`, with `SKIP_UI_BUILD` as the
/// CI escape hatch.
fn ensure_ui_assets() {
    println!("cargo:rerun-if-changed=ui/config-form.tsx");
    println!("cargo:rerun-if-changed=ui/catalog-page.tsx");
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
        ui_dir.join("dist").join("config-form.js"),
        ui_dir.join("dist").join("catalog-page.js"),
        ui_dir.join("dist").join("styles.css"),
    ];

    if dist_assets
        .iter()
        .all(|a| a.exists() && ui_dist_is_fresh(a, &ui_dir))
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

    for args in [["install"], ["build"]] {
        let status = Command::new(&pnpm)
            .args(args)
            .current_dir(&ui_dir)
            .status()
            .unwrap_or_else(|e| {
                panic!(
                    "failed to spawn `pnpm {}` in {}: {e}",
                    args[0],
                    ui_dir.display()
                )
            });
        if !status.success() {
            panic!("`pnpm {}` exited with {status} — see logs above", args[0]);
        }
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

/// `true` when the built ui asset is at least as new as every source that
/// contributes to it. Conservative: any I/O failure forces a rebuild.
fn ui_dist_is_fresh(dist_asset: &Path, ui_dir: &Path) -> bool {
    let Ok(dist_mtime) = dist_asset.metadata().and_then(|m| m.modified()) else {
        return false;
    };

    let watched_files = [
        ui_dir.join("config-form.tsx"),
        ui_dir.join("catalog-page.tsx"),
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

    let src = ui_dir.join("src");
    if src.exists() && !subtree_older_than(&src, dist_mtime) {
        return false;
    }

    true
}

/// `true` when the existing dist bundle is at least as new as every
/// source file that contributes to it. Conservative: any I/O failure or
/// missing mtime returns `false`, forcing a rebuild.
fn dist_is_fresh(dist_index: &Path, web_dir: &Path) -> bool {
    let Ok(dist_mtime) = dist_index.metadata().and_then(|m| m.modified()) else {
        return false;
    };

    let watched_dirs = [web_dir.join("src"), web_dir.join("public")];
    for dir in watched_dirs.iter() {
        if !dir.exists() {
            continue;
        }
        if !subtree_older_than(dir, dist_mtime) {
            return false;
        }
    }

    let watched_files = [
        web_dir.join("index.html"),
        web_dir.join("package.json"),
        web_dir.join("pnpm-lock.yaml"),
        web_dir.join("vite.config.ts"),
        web_dir.join(".env.production"),
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
        "pnpm not found on PATH — install Node + pnpm, or set SKIP_WEB_BUILD=1 \
         after building the SPA manually with `cd web && pnpm install && pnpm build`"
    );
}
