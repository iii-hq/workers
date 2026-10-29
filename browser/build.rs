//! Build script for the `browser` worker.
//!
//! 1. Forwards the build-time target triple to the binary as `env!("TARGET")`
//!    (used by `manifest.rs` for the registry `supported_targets` field).
//! 2. Ensures the embedded frontends exist and are fresh, since both are read
//!    with `include_str!`; when one is missing or older than its sources we run
//!    `pnpm install && pnpm build` inside its project first (the state
//!    worker's precedent):
//!    - `src/ui.rs` embeds the injected console UI, `ui/dist/page.js` and
//!      `ui/dist/styles.css`;
//!    - `src/native_ui.rs` embeds the native app for the iii app,
//!      `native-ui/dist/native.js` (which bundles the `@iii-dev/native-ui`
//!      SDK and the console UI's address rule, `ui/src/lib/address.ts`).
//!
//!    Set `SKIP_UI_BUILD=1` to use the existing `dist/` outputs of both as-is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// One pnpm project whose build output the worker embeds.
struct Frontend {
    /// The project directory, relative to the crate.
    dir: &'static str,
    /// Its outputs, relative to `dir`.
    assets: &'static [&'static str],
    /// Files whose change makes the outputs stale, relative to `dir`.
    files: &'static [&'static str],
    /// Directories whose change makes the outputs stale, relative to `dir`.
    dirs: &'static [&'static str],
}

const FRONTENDS: &[Frontend] = &[
    Frontend {
        dir: "ui",
        assets: &["dist/page.js", "dist/styles.css"],
        files: &[
            "page.tsx",
            "styles.css",
            "build.mjs",
            "package.json",
            "tsconfig.json",
            // The lockfile lives at the workers-repo root (pnpm workspace: the
            // ui project links @iii-dev/console-ui from packages/console-ui).
            "../../pnpm-lock.yaml",
        ],
        dirs: &["src"],
    },
    Frontend {
        dir: "native-ui",
        assets: &["dist/native.js"],
        files: &[
            "build.mjs",
            "package.json",
            "tsconfig.json",
            "../../pnpm-lock.yaml",
            // The address rule is the console UI's own (D10).
            "../ui/src/lib/address.ts",
            // The SDK is bundled in (not an import map like the console UI's).
            "../../packages/native-ui/catalog.json",
        ],
        dirs: &["src", "../../packages/native-ui/src"],
    },
];

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // `dist/` itself is never listed: include_str! reads it directly, and
    // listing it would rebuild-loop on our own output.
    for frontend in FRONTENDS {
        let dir = Path::new(frontend.dir);
        for watched in frontend.files.iter().chain(frontend.dirs) {
            println!("cargo:rerun-if-changed={}", dir.join(watched).display());
        }
    }

    let skip = std::env::var_os("SKIP_UI_BUILD").is_some();
    for frontend in FRONTENDS {
        ensure_built(&manifest_dir, frontend, skip);
    }
}

/// Builds `frontend` unless its outputs are at least as new as every source.
fn ensure_built(manifest_dir: &Path, frontend: &Frontend, skip: bool) {
    let dir = manifest_dir.join(frontend.dir);
    let assets: Vec<PathBuf> = frontend.assets.iter().map(|a| dir.join(a)).collect();

    if assets
        .iter()
        .all(|a| a.exists() && dist_is_fresh(a, &dir, frontend))
    {
        return;
    }

    if skip {
        for asset in &assets {
            if !asset.exists() {
                panic!(
                    "SKIP_UI_BUILD set but {} is missing — build it manually \
                     (cd {} && pnpm install && pnpm build) or unset the env var",
                    asset.display(),
                    frontend.dir
                );
            }
        }
        return;
    }

    let pnpm = locate_pnpm();

    let status = Command::new(&pnpm)
        .args(["install"])
        .current_dir(&dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `pnpm install` in {}: {e}", dir.display()));
    if !status.success() {
        panic!(
            "`pnpm install` in {} exited with {status} — see logs above",
            frontend.dir
        );
    }

    let status = Command::new(&pnpm)
        .args(["build"])
        .current_dir(&dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `pnpm build` in {}: {e}", dir.display()));
    if !status.success() {
        panic!(
            "`pnpm build` in {} exited with {status} — see logs above",
            frontend.dir
        );
    }

    for asset in &assets {
        if !asset.exists() {
            panic!(
                "`pnpm build` finished but {} is still missing — check the build \
                 output above",
                asset.display()
            );
        }
    }
}

/// `true` when the built asset is at least as new as every source that
/// contributes to it. Conservative: any I/O failure forces a rebuild.
fn dist_is_fresh(dist_asset: &Path, dir: &Path, frontend: &Frontend) -> bool {
    let Ok(dist_mtime) = dist_asset.metadata().and_then(|m| m.modified()) else {
        return false;
    };

    for f in frontend.files.iter().map(|f| dir.join(f)) {
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

    for d in frontend.dirs.iter().map(|d| dir.join(d)) {
        if d.exists() && !subtree_older_than(&d, dist_mtime) {
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
         after building both frontends manually with `pnpm install && pnpm build` \
         in ui/ and native-ui/"
    );
}
