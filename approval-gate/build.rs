//! Builds the React console extension before Rust embeds its output.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );

    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-changed=web/tsconfig.json");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let web_dir = manifest_dir.join("web");
    let extension_js = web_dir.join("dist/extension.js");
    let extension_css = web_dir.join("dist/extension.css");

    if extension_js.exists()
        && extension_css.exists()
        && dist_is_fresh(&extension_js, &web_dir)
        && dist_is_fresh(&extension_css, &web_dir)
    {
        return;
    }

    if std::env::var_os("SKIP_WEB_BUILD").is_some() {
        if !extension_js.exists() || !extension_css.exists() {
            panic!(
                "SKIP_WEB_BUILD set but the approval-gate extension bundle is missing — build it with `cd web && pnpm install && pnpm build`"
            );
        }
        return;
    }

    let pnpm = locate_pnpm();
    run_pnpm(&pnpm, &web_dir, &["install", "--frozen-lockfile"]);
    run_pnpm(&pnpm, &web_dir, &["build"]);

    if !extension_js.exists() || !extension_css.exists() {
        panic!("approval-gate web build finished without dist/extension.js and dist/extension.css");
    }
}

fn run_pnpm(pnpm: &Path, web_dir: &Path, args: &[&str]) {
    let status = Command::new(pnpm)
        .args(args)
        .current_dir(web_dir)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "failed to spawn `pnpm {}` in {}: {error}",
                args.join(" "),
                web_dir.display()
            )
        });
    if !status.success() {
        panic!("`pnpm {}` exited with {status}", args.join(" "));
    }
}

fn dist_is_fresh(output: &Path, web_dir: &Path) -> bool {
    let Ok(output_mtime) = output.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    if !subtree_older_than(&web_dir.join("src"), output_mtime) {
        return false;
    }
    for file in [
        "package.json",
        "pnpm-lock.yaml",
        "vite.config.ts",
        "tsconfig.json",
    ] {
        let path = web_dir.join(file);
        if !path.exists() {
            continue;
        }
        let Ok(modified) = path.metadata().and_then(|metadata| metadata.modified()) else {
            return false;
        };
        if modified > output_mtime {
            return false;
        }
    }
    true
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
        "pnpm not found on PATH — install Node + pnpm, or pre-build approval-gate/web and set SKIP_WEB_BUILD=1"
    );
}
